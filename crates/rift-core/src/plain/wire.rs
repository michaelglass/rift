//! The core's events and effects on plain data, as `u64` words: the Roc core's boundary (one
//! event in, one effect list out) and the recorded-trace format (event, effect list, …).
//!
//! Every field is one word: integers zero-extended (a pid sign-extended), a float as its bits,
//! an enum as its variant's index. An `Option` is a flag word (0 none, 1 some) and then the
//! value; a list is its length and then its items. Each event and effect starts with its tag,
//! the index in the tables below, which `roc/core` mirrors.
//!
//! A layout's shape and its frames cross the same way ([`encode_bsp`], [`encode_traditional`],
//! [`encode_frames`]): the Roc core's geometry entry takes a shape's words, which start with its
//! layout system's tag (0 BSP, 1 traditional), and returns the frames' words. A scrolling strip
//! (tag 2, [`encode_strip`]) returns where it placed everything ([`encode_placed`]); its column
//! ids are the platform's, one word each.

use crate::core::{Desire, Effect, Event, Plan};
use crate::layout::bsp::{Bsp, Children, Fill, Node, Slot};
use crate::layout::traditional::{self, StackLine, Traditional};
use crate::layout::{Gaps, Size, Sizing, WindowConstraints, scrolling};
use crate::plain::{Nanos, Plain, Rect, RequestId, WindowId};
use crate::pointer::{Button, Release, Ungrip};
use crate::reconciler::{Answer, Note, Reply, Want, Write};

/// What a word stream that is not a well-formed event or effect list breaks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireError {
    /// It ended inside a value.
    Truncated,
    /// A tag or enum index out of range, at this word.
    Unknown { at: usize, word: u64 },
    /// A value its type rules out (a size not finite, or negative), at this word.
    Invalid { at: usize, word: u64 },
}

// ---------------------------------------------------------------------------------------------
// Writing.

#[derive(Default)]
struct Words(Vec<u64>);

impl Words {
    fn word(&mut self, word: u64) { self.0.push(word) }

    fn window(&mut self, window: WindowId) {
        self.word(i64::from(window.pid) as u64);
        self.word(u64::from(window.idx));
    }

    fn frame(&mut self, frame: Rect) {
        for edge in [frame.x, frame.y, frame.w, frame.h] {
            self.word(edge.to_bits());
        }
    }

    fn request(&mut self, request: RequestId) { self.word(u64::from(request.0)) }

    fn option<T>(&mut self, value: Option<T>, mut some: impl FnMut(&mut Self, T)) {
        match value {
            None => self.word(0),
            Some(value) => {
                self.word(1);
                some(self, value)
            }
        }
    }

    fn list<T>(&mut self, items: Vec<T>, mut item: impl FnMut(&mut Self, T)) {
        self.word(items.len() as u64);
        for value in items {
            item(self, value);
        }
    }

    fn write(&mut self, write: Write<Plain>) {
        self.window(write.window());
        self.frame(write.frame());
        self.request(write.request());
    }

    fn desire(&mut self, desire: Desire<Plain>) {
        self.window(desire.window);
        self.frame(desire.frame);
        self.request(desire.request);
    }

    fn flag(&mut self, flag: bool) { self.word(u64::from(flag)) }

    fn float(&mut self, value: f64) { self.word(value.to_bits()) }

    fn gaps(&mut self, gaps: Gaps) {
        for gap in [
            gaps.top,
            gaps.left,
            gaps.bottom,
            gaps.right,
            gaps.horizontal,
            gaps.vertical,
        ] {
            self.float(gap);
        }
    }

    /// How tiling may size the window (0 fixed, 1 resizable, 2 resizable at least its
    /// minimum) and the minimum it reports, then its locks and maxima.
    fn constraints(&mut self, c: WindowConstraints) {
        let (tag, min) = match c.sizing {
            Sizing::Fixed { min } => (0, min),
            Sizing::Resizable { advisory_min } => (1, advisory_min),
            Sizing::ResizableAtLeastMin { min } => (2, min),
        };
        self.word(tag);
        for value in [
            min.width(),
            min.height(),
            c.locked_width,
            c.locked_height,
            c.max_width,
            c.max_height,
        ] {
            self.float(value);
        }
    }

    fn node(&mut self, node: Node<WindowId>) {
        match node {
            Node::Split { horizontal, ratio, children } => {
                self.word(0);
                self.flag(horizontal);
                self.word(u64::from(ratio.to_bits()));
                match children {
                    Children::One(child) => {
                        self.word(0);
                        self.node(*child);
                    }
                    Children::Two(first, second) => {
                        self.word(1);
                        self.node(*first);
                        self.node(*second);
                    }
                }
            }
            Node::Leaf { slot, fill, stacked } => {
                self.word(1);
                match slot {
                    Slot::Empty => self.word(0),
                    Slot::Window { window, constraints } => {
                        self.word(1);
                        self.window(window);
                        self.option(constraints, Self::constraints);
                    }
                }
                self.word(match fill {
                    Fill::Tiled => 0,
                    Fill::Screen => 1,
                    Fill::WithinGaps => 2,
                });
                self.flag(stacked);
            }
        }
    }

    fn fill(&mut self, fill: traditional::Fill) {
        self.word(match fill {
            traditional::Fill::Tiled => 0,
            traditional::Fill::Screen => 1,
            traditional::Fill::WithinGaps => 2,
        });
    }

    fn child(&mut self, child: traditional::Child<WindowId>) {
        self.word(u64::from(child.size.to_bits()));
        self.tnode(child.node);
    }

    fn tnode(&mut self, node: traditional::Node<WindowId>) {
        match node {
            traditional::Node::Window { window, constraints, fill } => {
                self.word(0);
                self.window(window);
                self.option(constraints, Self::constraints);
                self.fill(fill);
            }
            traditional::Node::Axis { horizontal, fill, first, rest } => {
                self.word(1);
                self.flag(horizontal);
                self.fill(fill);
                self.child(*first);
                self.list(rest, Self::child);
            }
            traditional::Node::Stack {
                horizontal,
                fill,
                before,
                focused,
                after,
            } => {
                self.word(2);
                self.flag(horizontal);
                self.fill(fill);
                self.list(before, Self::child);
                self.child(*focused);
                self.list(after, Self::child);
            }
        }
    }

    fn column(&mut self, column: scrolling::Column<WindowId, u64>) {
        self.word(column.id);
        match column.width {
            scrolling::Width::Default => self.word(0),
            scrolling::Width::Proportion(ratio) => {
                self.word(1);
                self.float(ratio);
            }
            scrolling::Width::Fixed(width) => {
                self.word(2);
                self.float(width);
            }
        }
        self.row(column.first);
        self.list(column.rest, Words::row);
    }

    fn row(&mut self, row: scrolling::Row<WindowId>) {
        self.window(row.window);
        self.constraints(row.constraints);
        self.float(row.weight);
    }

    fn spot(&mut self, spot: scrolling::Spot<u64>) {
        self.word(spot.id);
        self.float(spot.world_x);
        self.float(spot.width);
        self.float(spot.anchor);
    }

    fn snap(&mut self, snap: scrolling::Snap<u64>) {
        self.float(snap.offset);
        self.word(snap.column);
    }

    fn event(&mut self, event: Event<Plain>) {
        match event {
            Event::Press { window, button } => {
                self.word(0);
                self.option(window, Self::window);
                self.word(button as u64);
            }
            Event::Drag { window } => {
                self.word(1);
                self.window(window);
            }
            Event::Release(release) => {
                self.word(2);
                self.word(match release {
                    Release::Button(Button::Left) => 0,
                    Release::Button(Button::Right) => 1,
                    Release::Missed => 2,
                });
            }
            Event::LetGo => self.word(3),
            Event::Gone { window } => {
                self.word(4);
                self.window(window);
            }
            Event::Rekeyed { from, to } => {
                self.word(5);
                self.window(from);
                self.window(to);
            }
            Event::Seen { window, frame } => {
                self.word(6);
                self.window(window);
                self.frame(frame);
            }
            Event::Ceded { window } => {
                self.word(7);
                self.window(window);
            }
            Event::Unseen { window } => {
                self.word(8);
                self.window(window);
            }
            Event::Distrust { windows } => {
                self.word(9);
                self.list(windows, Self::window);
            }
            Event::Arrange { now, want, desires } => {
                self.word(10);
                self.word(now.0);
                self.word(want as u64);
                self.list(desires, Self::desire);
            }
            Event::Animate { now, desires } => {
                self.word(11);
                self.word(now.0);
                self.list(desires, Self::desire);
            }
            Event::AnimationEnded { now, request } => {
                self.word(12);
                self.word(now.0);
                self.request(request);
            }
            Event::AnimationStopped { window, request, at } => {
                self.word(13);
                self.window(window);
                self.request(request);
                self.option(at, Self::frame);
            }
            Event::Follow { request, window, frame } => {
                self.word(14);
                self.request(request);
                self.window(window);
                self.frame(frame);
            }
            Event::Observe {
                now,
                request,
                window,
                frame,
                as_of,
            } => {
                self.word(15);
                self.word(now.0);
                self.request(request);
                self.window(window);
                self.frame(frame);
                self.option(as_of, Self::request);
            }
            Event::Tick { now, request } => {
                self.word(16);
                self.word(now.0);
                self.request(request);
            }
            Event::Forget { window } => {
                self.word(17);
                self.window(window);
            }
            Event::SentOutside { window, request } => {
                self.word(18);
                self.window(window);
                self.request(request);
            }
        }
    }

    fn effect(&mut self, effect: Effect<Plain>) {
        match effect {
            Effect::Write(write) => {
                self.word(0);
                self.write(write);
            }
            Effect::Animate { window, from, to, request } => {
                self.word(1);
                self.window(window);
                self.frame(from);
                self.frame(to);
                self.request(request);
            }
            Effect::Pending { window, target } => {
                self.word(2);
                self.window(window);
                self.option(target, Self::frame);
            }
            Effect::Plan(Plan { request, positions, frames }) => {
                self.word(3);
                self.request(request);
                self.list(positions, Self::write);
                self.list(frames, Self::write);
            }
            Effect::Queued { window } => {
                self.word(4);
                self.window(window);
            }
            Effect::Seen { window, frame } => {
                self.word(5);
                self.window(window);
                self.frame(frame);
            }
            Effect::Replied { window, reply } => {
                self.word(6);
                self.window(window);
                self.word(match reply {
                    Reply::NoNews => 0,
                    Reply::InTransit => 1,
                    Reply::Answered(Answer::Acked) => 2,
                    Reply::Answered(Answer::Refused) => 3,
                    Reply::Foreign => 4,
                });
            }
            Effect::Note { window, note } => {
                self.word(7);
                self.window(window);
                self.word(note as u64);
            }
            Effect::Timer(at) => {
                self.word(8);
                self.option(at, |words, at| words.word(at.0));
            }
            Effect::Grip { window } => {
                self.word(9);
                self.window(window);
            }
            Effect::DragBegan { window } => {
                self.word(10);
                self.window(window);
            }
            Effect::Ungrip { window, why, changed } => {
                self.word(11);
                self.window(window);
                self.word(why as u64);
                self.list(changed, Self::window);
            }
            Effect::Regrip { from, to } => {
                self.word(12);
                self.window(from);
                self.window(to);
            }
        }
    }
}

/// `event` as words.
pub fn encode_event(event: Event<Plain>) -> Vec<u64> {
    let mut words = Vec::new();
    encode_event_into(&mut words, event);
    words
}

/// `event` as words, into `words` (cleared first; its allocation is kept).
pub fn encode_event_into(words: &mut Vec<u64>, event: Event<Plain>) {
    let mut writer = Words(std::mem::take(words));
    writer.0.clear();
    writer.event(event);
    *words = writer.0;
}

/// `effects` as words.
pub fn encode_effects(effects: Vec<Effect<Plain>>) -> Vec<u64> {
    let mut words = Words::default();
    words.list(effects, Words::effect);
    words.0
}

/// A BSP layout's shape as words.
pub fn encode_bsp(bsp: Bsp<WindowId>) -> Vec<u64> {
    let mut words = Words::default();
    words.word(0);
    words.frame(bsp.screen);
    words.gaps(bsp.gaps);
    words.option(bsp.aspect, Words::float);
    words.node(bsp.root);
    words.0
}

/// A traditional layout's shape as words.
pub fn encode_traditional(shape: Traditional<WindowId>) -> Vec<u64> {
    let mut words = Words::default();
    words.word(1);
    words.frame(shape.screen);
    words.frame(shape.area);
    words.gaps(shape.gaps);
    words.float(shape.stack_offset);
    let line = shape.stack_line;
    words.float(line.thickness);
    words.float(line.spacing);
    words.flag(line.top);
    words.flag(line.left);
    words.tnode(shape.root);
    words.0
}

/// A scrolling strip as words.
pub fn encode_strip(strip: scrolling::Strip<WindowId, u64>) -> Vec<u64> {
    let mut words = Words::default();
    words.word(2);
    words.frame(strip.tiling);
    words.float(strip.column_gap);
    words.float(strip.row_gap);
    let widths = strip.widths;
    words.float(widths.ratio);
    words.float(widths.min_ratio);
    words.float(widths.max_ratio);
    words.flag(widths.preserve_window_sizes);
    words.flag(widths.expand_single_column);
    words.word(match strip.alignment {
        scrolling::Alignment::Left => 0,
        scrolling::Alignment::Center => 1,
        scrolling::Alignment::Right => 2,
    });
    words.word(match strip.navigation {
        scrolling::Navigation::Anchored => 0,
        scrolling::Navigation::Free => 1,
    });
    words.column(strip.first);
    words.list(strip.rest, Words::column);
    words.0
}

/// Where a strip placed everything, as words: its first column's spot and the rest, the
/// frames, the first snap and the rest, the bounds.
pub fn encode_placed(placed: scrolling::Placed<WindowId, u64>) -> Vec<u64> {
    let mut words = Words::default();
    words.spot(placed.first);
    words.list(placed.rest, Words::spot);
    words.list(placed.frames, |words, (window, frame)| {
        words.window(window);
        words.frame(frame);
    });
    words.snap(placed.snaps.0);
    words.list(placed.snaps.1, Words::snap);
    words.float(placed.bounds.0);
    words.float(placed.bounds.1);
    words.0
}

/// A layout's frames (each window's, in order) as words.
pub fn encode_frames(frames: Vec<(WindowId, Rect)>) -> Vec<u64> {
    let mut words = Words::default();
    words.list(frames, |words, (window, frame)| {
        words.window(window);
        words.frame(frame);
    });
    words.0
}

/// One step of a trace (its event, then its effects), appended to `trace`.
pub fn record_step(trace: &mut Vec<u64>, event: Event<Plain>, effects: Vec<Effect<Plain>>) {
    trace.extend(encode_event(event));
    trace.extend(encode_effects(effects));
}

// ---------------------------------------------------------------------------------------------
// Reading.

struct Reader<'a> {
    words: &'a [u64],
    at: usize,
}

type Read<T> = Result<T, WireError>;

impl Reader<'_> {
    fn word(&mut self) -> Read<u64> {
        let word = *self.words.get(self.at).ok_or(WireError::Truncated)?;
        self.at += 1;
        Ok(word)
    }

    fn unknown<T>(&self, word: u64) -> Read<T> { Err(WireError::Unknown { at: self.at - 1, word }) }

    fn u32(&mut self) -> Read<u32> {
        let word = self.word()?;
        u32::try_from(word).or_else(|_| self.unknown(word))
    }

    fn window(&mut self) -> Read<WindowId> {
        let pid = self.word()?;
        let pid = i32::try_from(pid as i64).or_else(|_| self.unknown(pid))?;
        Ok(WindowId { pid, idx: self.u32()? })
    }

    fn frame(&mut self) -> Read<Rect> {
        let mut edge = || self.word().map(f64::from_bits);
        Ok(Rect {
            x: edge()?,
            y: edge()?,
            w: edge()?,
            h: edge()?,
        })
    }

    fn request(&mut self) -> Read<RequestId> { Ok(RequestId(self.u32()?)) }

    fn time(&mut self) -> Read<Nanos> { Ok(Nanos(self.word()?)) }

    fn option<T>(&mut self, mut some: impl FnMut(&mut Self) -> Read<T>) -> Read<Option<T>> {
        match self.word()? {
            0 => Ok(None),
            1 => some(self).map(Some),
            word => self.unknown(word),
        }
    }

    fn list<T>(&mut self, mut item: impl FnMut(&mut Self) -> Read<T>) -> Read<Vec<T>> {
        let length = self.word()?;
        // Each item takes at least one word: a length past the end is truncation, not a
        // reason to allocate.
        if length > (self.words.len() - self.at) as u64 {
            return Err(WireError::Truncated);
        }
        (0..length).map(|_| item(self)).collect()
    }

    fn write(&mut self) -> Read<Write<Plain>> {
        Ok(Write::new(self.window()?, self.frame()?, self.request()?))
    }

    fn desire(&mut self) -> Read<Desire<Plain>> {
        Ok(Desire {
            window: self.window()?,
            frame: self.frame()?,
            request: self.request()?,
        })
    }

    fn flag(&mut self) -> Read<bool> {
        match self.word()? {
            0 => Ok(false),
            1 => Ok(true),
            word => self.unknown(word),
        }
    }

    fn float(&mut self) -> Read<f64> { self.word().map(f64::from_bits) }

    fn gaps(&mut self) -> Read<Gaps> {
        Ok(Gaps {
            top: self.float()?,
            left: self.float()?,
            bottom: self.float()?,
            right: self.float()?,
            horizontal: self.float()?,
            vertical: self.float()?,
        })
    }

    fn constraints(&mut self) -> Read<WindowConstraints> {
        let sizing: fn(Size) -> Sizing = match self.word()? {
            0 => |min| Sizing::Fixed { min },
            1 => |advisory_min| Sizing::Resizable { advisory_min },
            2 => |min| Sizing::ResizableAtLeastMin { min },
            word => return self.unknown(word),
        };
        Ok(WindowConstraints {
            sizing: sizing(self.size()?),
            locked_width: self.float()?,
            locked_height: self.float()?,
            max_width: self.float()?,
            max_height: self.float()?,
        })
    }

    /// A size as the core holds one, already parsed: each axis finite and not negative.
    fn size(&mut self) -> Read<Size> {
        let mut axis = || {
            let word = self.word()?;
            let value = f64::from_bits(word);
            if value.is_finite() && value >= 0.0 {
                Ok(value)
            } else {
                Err(WireError::Invalid { at: self.at - 1, word })
            }
        };
        let (width, height) = (axis()?, axis()?);
        Ok(Size::reported(width, height).unwrap_or(Size::ZERO))
    }

    fn node(&mut self) -> Read<Node<WindowId>> {
        match self.word()? {
            0 => {
                let horizontal = self.flag()?;
                let ratio = f32::from_bits(self.u32()?);
                let children = match self.word()? {
                    0 => Children::One(Box::new(self.node()?)),
                    1 => Children::Two(Box::new(self.node()?), Box::new(self.node()?)),
                    word => return self.unknown(word),
                };
                Ok(Node::Split { horizontal, ratio, children })
            }
            1 => {
                let slot = match self.word()? {
                    0 => Slot::Empty,
                    1 => Slot::Window {
                        window: self.window()?,
                        constraints: self.option(Self::constraints)?,
                    },
                    word => return self.unknown(word),
                };
                let fill = match self.word()? {
                    0 => Fill::Tiled,
                    1 => Fill::Screen,
                    2 => Fill::WithinGaps,
                    word => return self.unknown(word),
                };
                Ok(Node::Leaf {
                    slot,
                    fill,
                    stacked: self.flag()?,
                })
            }
            word => self.unknown(word),
        }
    }

    fn fill(&mut self) -> Read<traditional::Fill> {
        match self.word()? {
            0 => Ok(traditional::Fill::Tiled),
            1 => Ok(traditional::Fill::Screen),
            2 => Ok(traditional::Fill::WithinGaps),
            word => self.unknown(word),
        }
    }

    fn child(&mut self) -> Read<traditional::Child<WindowId>> {
        Ok(traditional::Child {
            size: f32::from_bits(self.u32()?),
            node: self.tnode()?,
        })
    }

    fn tnode(&mut self) -> Read<traditional::Node<WindowId>> {
        match self.word()? {
            0 => Ok(traditional::Node::Window {
                window: self.window()?,
                constraints: self.option(Self::constraints)?,
                fill: self.fill()?,
            }),
            1 => Ok(traditional::Node::Axis {
                horizontal: self.flag()?,
                fill: self.fill()?,
                first: Box::new(self.child()?),
                rest: self.list(Self::child)?,
            }),
            2 => Ok(traditional::Node::Stack {
                horizontal: self.flag()?,
                fill: self.fill()?,
                before: self.list(Self::child)?,
                focused: Box::new(self.child()?),
                after: self.list(Self::child)?,
            }),
            word => self.unknown(word),
        }
    }

    fn column(&mut self) -> Read<scrolling::Column<WindowId, u64>> {
        let id = self.word()?;
        let width = match self.word()? {
            0 => scrolling::Width::Default,
            1 => scrolling::Width::Proportion(self.float()?),
            2 => scrolling::Width::Fixed(self.float()?),
            word => return self.unknown(word),
        };
        Ok(scrolling::Column {
            id,
            width,
            first: self.row()?,
            rest: self.list(Reader::row)?,
        })
    }

    fn row(&mut self) -> Read<scrolling::Row<WindowId>> {
        Ok(scrolling::Row {
            window: self.window()?,
            constraints: self.constraints()?,
            weight: self.float()?,
        })
    }

    fn spot(&mut self) -> Read<scrolling::Spot<u64>> {
        Ok(scrolling::Spot {
            id: self.word()?,
            world_x: self.float()?,
            width: self.float()?,
            anchor: self.float()?,
        })
    }

    fn snap(&mut self) -> Read<scrolling::Snap<u64>> {
        Ok(scrolling::Snap {
            offset: self.float()?,
            column: self.word()?,
        })
    }

    fn button(&mut self) -> Read<Button> {
        match self.word()? {
            0 => Ok(Button::Left),
            1 => Ok(Button::Right),
            word => self.unknown(word),
        }
    }

    fn event(&mut self) -> Read<Event<Plain>> {
        Ok(match self.word()? {
            0 => Event::Press {
                window: self.option(Self::window)?,
                button: self.button()?,
            },
            1 => Event::Drag { window: self.window()? },
            2 => Event::Release(match self.word()? {
                0 => Release::Button(Button::Left),
                1 => Release::Button(Button::Right),
                2 => Release::Missed,
                word => return self.unknown(word),
            }),
            3 => Event::LetGo,
            4 => Event::Gone { window: self.window()? },
            5 => Event::Rekeyed {
                from: self.window()?,
                to: self.window()?,
            },
            6 => Event::Seen {
                window: self.window()?,
                frame: self.frame()?,
            },
            7 => Event::Ceded { window: self.window()? },
            8 => Event::Unseen { window: self.window()? },
            9 => Event::Distrust {
                windows: self.list(Self::window)?,
            },
            10 => Event::Arrange {
                now: self.time()?,
                want: match self.word()? {
                    0 => Want::Arrange,
                    1 => Want::ArrangePositions,
                    word => return self.unknown(word),
                },
                desires: self.list(Self::desire)?,
            },
            11 => Event::Animate {
                now: self.time()?,
                desires: self.list(Self::desire)?,
            },
            12 => Event::AnimationEnded {
                now: self.time()?,
                request: self.request()?,
            },
            13 => Event::AnimationStopped {
                window: self.window()?,
                request: self.request()?,
                at: self.option(Self::frame)?,
            },
            14 => Event::Follow {
                request: self.request()?,
                window: self.window()?,
                frame: self.frame()?,
            },
            15 => Event::Observe {
                now: self.time()?,
                request: self.request()?,
                window: self.window()?,
                frame: self.frame()?,
                as_of: self.option(Self::request)?,
            },
            16 => Event::Tick {
                now: self.time()?,
                request: self.request()?,
            },
            17 => Event::Forget { window: self.window()? },
            18 => Event::SentOutside {
                window: self.window()?,
                request: self.request()?,
            },
            word => return self.unknown(word),
        })
    }

    fn effect(&mut self) -> Read<Effect<Plain>> {
        Ok(match self.word()? {
            0 => Effect::Write(self.write()?),
            1 => Effect::Animate {
                window: self.window()?,
                from: self.frame()?,
                to: self.frame()?,
                request: self.request()?,
            },
            2 => Effect::Pending {
                window: self.window()?,
                target: self.option(Self::frame)?,
            },
            3 => Effect::Plan(Plan {
                request: self.request()?,
                positions: self.list(Self::write)?,
                frames: self.list(Self::write)?,
            }),
            4 => Effect::Queued { window: self.window()? },
            5 => Effect::Seen {
                window: self.window()?,
                frame: self.frame()?,
            },
            6 => Effect::Replied {
                window: self.window()?,
                reply: match self.word()? {
                    0 => Reply::NoNews,
                    1 => Reply::InTransit,
                    2 => Reply::Answered(Answer::Acked),
                    3 => Reply::Answered(Answer::Refused),
                    4 => Reply::Foreign,
                    word => return self.unknown(word),
                },
            },
            7 => Effect::Note {
                window: self.window()?,
                note: match self.word()? {
                    0 => Note::Discarded,
                    1 => Note::TimedOut,
                    word => return self.unknown(word),
                },
            },
            8 => Effect::Timer(self.option(Self::time)?),
            9 => Effect::Grip { window: self.window()? },
            10 => Effect::DragBegan { window: self.window()? },
            11 => Effect::Ungrip {
                window: self.window()?,
                why: match self.word()? {
                    0 => Ungrip::Released,
                    1 => Ungrip::LetGo,
                    2 => Ungrip::Gone,
                    word => return self.unknown(word),
                },
                changed: self.list(Self::window)?,
            },
            12 => Effect::Regrip {
                from: self.window()?,
                to: self.window()?,
            },
            word => return self.unknown(word),
        })
    }

    fn done(&self) -> Read<()> {
        match self.words.get(self.at) {
            None => Ok(()),
            Some(&word) => Err(WireError::Unknown { at: self.at, word }),
        }
    }
}

/// The event `words` hold, all of them.
pub fn decode_event(words: &[u64]) -> Result<Event<Plain>, WireError> {
    let mut reader = Reader { words, at: 0 };
    let event = reader.event()?;
    reader.done()?;
    Ok(event)
}

/// The effect list `words` hold, all of them.
pub fn decode_effects(words: &[u64]) -> Result<Vec<Effect<Plain>>, WireError> {
    let mut reader = Reader { words, at: 0 };
    let effects = reader.list(Reader::effect)?;
    reader.done()?;
    Ok(effects)
}

/// The BSP shape `words` hold.
pub fn decode_bsp(words: &[u64]) -> Result<Bsp<WindowId>, WireError> {
    let mut reader = Reader { words, at: 0 };
    match reader.word()? {
        0 => {}
        word => return reader.unknown(word),
    }
    let bsp = Bsp {
        screen: reader.frame()?,
        gaps: reader.gaps()?,
        aspect: reader.option(Reader::float)?,
        root: reader.node()?,
    };
    reader.done()?;
    Ok(bsp)
}

/// The traditional shape `words` hold.
pub fn decode_traditional(words: &[u64]) -> Result<Traditional<WindowId>, WireError> {
    let mut reader = Reader { words, at: 0 };
    match reader.word()? {
        1 => {}
        word => return reader.unknown(word),
    }
    let shape = Traditional {
        screen: reader.frame()?,
        area: reader.frame()?,
        gaps: reader.gaps()?,
        stack_offset: reader.float()?,
        stack_line: StackLine {
            thickness: reader.float()?,
            spacing: reader.float()?,
            top: reader.flag()?,
            left: reader.flag()?,
        },
        root: reader.tnode()?,
    };
    reader.done()?;
    Ok(shape)
}

/// The scrolling strip `words` hold.
pub fn decode_strip(words: &[u64]) -> Result<scrolling::Strip<WindowId, u64>, WireError> {
    let mut reader = Reader { words, at: 0 };
    match reader.word()? {
        2 => {}
        word => return reader.unknown(word),
    }
    let tiling = reader.frame()?;
    let column_gap = reader.float()?;
    let row_gap = reader.float()?;
    let widths = scrolling::Widths {
        ratio: reader.float()?,
        min_ratio: reader.float()?,
        max_ratio: reader.float()?,
        preserve_window_sizes: reader.flag()?,
        expand_single_column: reader.flag()?,
    };
    let alignment = match reader.word()? {
        0 => scrolling::Alignment::Left,
        1 => scrolling::Alignment::Center,
        2 => scrolling::Alignment::Right,
        word => return reader.unknown(word),
    };
    let navigation = match reader.word()? {
        0 => scrolling::Navigation::Anchored,
        1 => scrolling::Navigation::Free,
        word => return reader.unknown(word),
    };
    let strip = scrolling::Strip {
        tiling,
        column_gap,
        row_gap,
        widths,
        alignment,
        navigation,
        first: reader.column()?,
        rest: reader.list(Reader::column)?,
    };
    reader.done()?;
    Ok(strip)
}

/// Where a strip placed everything, as `words` hold it.
pub fn decode_placed(words: &[u64]) -> Result<scrolling::Placed<WindowId, u64>, WireError> {
    let mut reader = Reader { words, at: 0 };
    let placed = scrolling::Placed {
        first: reader.spot()?,
        rest: reader.list(Reader::spot)?,
        frames: reader.list(|reader| Ok((reader.window()?, reader.frame()?)))?,
        snaps: (reader.snap()?, reader.list(Reader::snap)?),
        bounds: (reader.float()?, reader.float()?),
    };
    reader.done()?;
    Ok(placed)
}

/// The frames `words` hold.
pub fn decode_frames(words: &[u64]) -> Result<Vec<(WindowId, Rect)>, WireError> {
    let mut reader = Reader { words, at: 0 };
    let frames = reader.list(|reader| Ok((reader.window()?, reader.frame()?)))?;
    reader.done()?;
    Ok(frames)
}

/// A recorded trace: each step's event and the effects the core decided.
pub fn decode_trace(words: &[u64]) -> Result<Vec<(Event<Plain>, Vec<Effect<Plain>>)>, WireError> {
    let mut reader = Reader { words, at: 0 };
    let mut steps = Vec::new();
    while reader.at < words.len() {
        steps.push((reader.event()?, reader.list(Reader::effect)?));
    }
    Ok(steps)
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: WindowId = WindowId { pid: -7, idx: 3 };
    const V: WindowId = WindowId { pid: 9, idx: u32::MAX };
    const R: RequestId = RequestId(42);

    fn at(x: f64) -> Rect {
        Rect {
            x,
            y: -0.5,
            w: 400.25,
            h: f64::MIN_POSITIVE,
        }
    }

    fn desire() -> Desire<Plain> {
        Desire {
            window: W,
            frame: at(1.),
            request: R,
        }
    }

    #[test]
    fn every_event_survives_the_wire() {
        let events = [
            Event::Press {
                window: Some(W),
                button: Button::Right,
            },
            Event::Press {
                window: None,
                button: Button::Left,
            },
            Event::Drag { window: W },
            Event::Release(Release::Button(Button::Right)),
            Event::Release(Release::Missed),
            Event::LetGo,
            Event::Gone { window: W },
            Event::Rekeyed { from: W, to: V },
            Event::Seen { window: W, frame: at(2.) },
            Event::Ceded { window: V },
            Event::Unseen { window: W },
            Event::Distrust { windows: vec![W, V] },
            Event::Arrange {
                now: Nanos(5),
                want: Want::ArrangePositions,
                desires: vec![desire()],
            },
            Event::Animate {
                now: Nanos(6),
                desires: vec![desire(), desire()],
            },
            Event::AnimationEnded { now: Nanos(7), request: R },
            Event::AnimationStopped {
                window: W,
                request: R,
                at: Some(at(3.)),
            },
            Event::AnimationStopped {
                window: W,
                request: R,
                at: None,
            },
            Event::Follow {
                request: R,
                window: V,
                frame: at(4.),
            },
            Event::Observe {
                now: Nanos(8),
                request: R,
                window: W,
                frame: at(5.),
                as_of: Some(R),
            },
            Event::Observe {
                now: Nanos(8),
                request: R,
                window: W,
                frame: at(5.),
                as_of: None,
            },
            Event::Tick {
                now: Nanos(u64::MAX),
                request: R,
            },
            Event::Forget { window: W },
            Event::SentOutside {
                window: V,
                request: RequestId(9),
            },
        ];
        for event in events {
            assert_eq!(decode_event(&encode_event(event.clone())), Ok(event));
        }
    }

    #[test]
    fn every_effect_survives_the_wire() {
        let write = Write::new(W, at(1.), R);
        let effects = vec![
            Effect::Write(write),
            Effect::Animate {
                window: W,
                from: at(1.),
                to: at(2.),
                request: R,
            },
            Effect::Pending {
                window: W,
                target: Some(at(3.)),
            },
            Effect::Pending { window: V, target: None },
            Effect::Plan(Plan {
                request: R,
                positions: vec![write],
                frames: vec![write, write],
            }),
            Effect::Queued { window: W },
            Effect::Seen { window: W, frame: at(4.) },
            Effect::Replied {
                window: W,
                reply: Reply::NoNews,
            },
            Effect::Replied {
                window: W,
                reply: Reply::InTransit,
            },
            Effect::Replied {
                window: W,
                reply: Reply::Answered(Answer::Acked),
            },
            Effect::Replied {
                window: W,
                reply: Reply::Answered(Answer::Refused),
            },
            Effect::Replied {
                window: W,
                reply: Reply::Foreign,
            },
            Effect::Note {
                window: W,
                note: Note::Discarded,
            },
            Effect::Note {
                window: W,
                note: Note::TimedOut,
            },
            Effect::Timer(Some(Nanos(9))),
            Effect::Timer(None),
            Effect::Grip { window: W },
            Effect::DragBegan { window: W },
            Effect::Ungrip {
                window: W,
                why: Ungrip::Gone,
                changed: vec![V],
            },
            Effect::Regrip { from: W, to: V },
        ];
        assert_eq!(decode_effects(&encode_effects(effects.clone())), Ok(effects));
    }

    #[test]
    fn a_trace_reads_back_step_by_step_and_garbage_is_refused() {
        let mut trace = Vec::new();
        record_step(&mut trace, Event::LetGo, vec![Effect::Timer(None)]);
        record_step(&mut trace, Event::Forget { window: W }, vec![]);
        assert_eq!(
            decode_trace(&trace),
            Ok(vec![
                (Event::LetGo, vec![Effect::Timer(None)]),
                (Event::Forget { window: W }, vec![]),
            ])
        );
        assert_eq!(decode_event(&[99]), Err(WireError::Unknown { at: 0, word: 99 }));
        assert_eq!(decode_effects(&[u64::MAX]), Err(WireError::Truncated));
        assert_eq!(decode_event(&[4, 1]), Err(WireError::Truncated));
    }

    #[test]
    fn every_sizing_survives_the_wire_and_a_size_the_core_could_not_hold_is_refused() {
        let min = Size::reported(300., 120.);
        let shape = |sizing| Bsp {
            screen: Rect { x: 0., y: 0., w: 10., h: 10. },
            gaps: Gaps::default(),
            root: Node::Leaf {
                slot: Slot::Window {
                    window: W,
                    constraints: Some(WindowConstraints { sizing, ..Default::default() }),
                },
                fill: Fill::Tiled,
                stacked: false,
            },
            aspect: None,
        };
        for sizing in [
            Sizing::new(false, false, min),
            Sizing::new(true, false, min),
            Sizing::new(true, true, min),
            Sizing::new(true, false, None),
        ] {
            assert_eq!(decode_bsp(&encode_bsp(shape(sizing))), Ok(shape(sizing)));
        }
        let words = encode_bsp(shape(Sizing::new(true, true, min)));
        let width = words.iter().position(|&w| w == 300f64.to_bits()).unwrap();
        for bad in [-1f64, f64::NAN, f64::INFINITY] {
            let mut garbled = words.clone();
            garbled[width] = bad.to_bits();
            assert_eq!(
                decode_bsp(&garbled),
                Err(WireError::Invalid { at: width, word: bad.to_bits() })
            );
        }
        let mut garbled = words;
        garbled[width - 1] = 7;
        assert_eq!(
            decode_bsp(&garbled),
            Err(WireError::Unknown { at: width - 1, word: 7 })
        );
    }

    #[test]
    fn a_bsp_shape_and_its_frames_survive_the_wire() {
        let leaf = |window, constraints, fill| Node::Leaf {
            slot: Slot::Window { window, constraints },
            fill,
            stacked: false,
        };
        let bsp = Bsp {
            screen: Rect {
                x: -1.5,
                y: 2.,
                w: 1000.,
                h: 800.,
            },
            gaps: Gaps {
                top: 1.,
                left: 2.,
                bottom: 3.,
                right: 4.,
                horizontal: 5.,
                vertical: 6.,
            },
            aspect: Some(1.5),
            root: Node::Split {
                horizontal: true,
                ratio: 0.3,
                children: Children::Two(
                    Box::new(leaf(W, None, Fill::Screen)),
                    Box::new(Node::Split {
                        horizontal: false,
                        ratio: 0.5,
                        children: Children::One(Box::new(Node::Leaf {
                            slot: Slot::Empty,
                            fill: Fill::WithinGaps,
                            stacked: true,
                        })),
                    }),
                ),
            },
        };
        let mut constrained = bsp.clone();
        constrained.root = leaf(
            V,
            Some(WindowConstraints {
                sizing: Sizing::new(true, true, Size::reported(300., 0.)),
                ..Default::default()
            }),
            Fill::Tiled,
        );
        for shape in [bsp, constrained] {
            assert_eq!(decode_bsp(&encode_bsp(shape.clone())), Ok(shape));
        }
        let frames = vec![
            (W, Rect { x: 0., y: 1., w: 2., h: 3. }),
            (V, Rect { x: 4., y: 5., w: 6., h: 7. }),
        ];
        assert_eq!(decode_frames(&encode_frames(frames.clone())), Ok(frames));
    }

    #[test]
    fn a_traditional_shape_survives_the_wire() {
        use crate::layout::traditional::{Child, Fill, Node};
        let window = |window, fill| Node::Window {
            window,
            constraints: None,
            fill,
        };
        let child = |size, node| Child { size, node };
        let shape = Traditional {
            screen: Rect {
                x: 0.,
                y: 0.,
                w: 1000.,
                h: 800.,
            },
            area: Rect { x: 5., y: 5., w: 990., h: 790. },
            gaps: Gaps {
                top: 5.,
                left: 5.,
                bottom: 5.,
                right: 5.,
                horizontal: 4.,
                vertical: 3.,
            },
            stack_offset: 30.,
            stack_line: StackLine {
                thickness: 4.,
                spacing: 2.,
                top: true,
                left: false,
            },
            root: Node::Axis {
                horizontal: true,
                fill: Fill::Tiled,
                first: Box::new(child(1.5, window(W, Fill::Screen))),
                rest: vec![child(0.5, Node::Stack {
                    horizontal: false,
                    fill: Fill::WithinGaps,
                    before: vec![child(1., window(V, Fill::Tiled))],
                    focused: Box::new(child(1., Node::Window {
                        window: W,
                        constraints: Some(WindowConstraints {
                            max_height: 300.,
                            ..Default::default()
                        }),
                        fill: Fill::Tiled,
                    })),
                    after: vec![],
                })],
            },
        };
        assert_eq!(decode_traditional(&encode_traditional(shape.clone())), Ok(shape));
    }

    #[test]
    fn a_scrolling_strip_and_where_it_placed_everything_survive_the_wire() {
        let row = |window, weight| scrolling::Row {
            window,
            constraints: WindowConstraints {
                sizing: Sizing::new(true, false, Size::reported(200., 100.)),
                max_width: 900.,
                ..Default::default()
            },
            weight,
        };
        let column = |id, width, first, rest| scrolling::Column { id, width, first, rest };
        let strip = scrolling::Strip {
            tiling: Rect {
                x: 0.,
                y: 25.,
                w: 1200.,
                h: 800.,
            },
            column_gap: 8.,
            row_gap: 6.,
            widths: scrolling::Widths {
                ratio: 0.5,
                min_ratio: 0.2,
                max_ratio: 0.9,
                preserve_window_sizes: true,
                expand_single_column: false,
            },
            alignment: scrolling::Alignment::Right,
            navigation: scrolling::Navigation::Free,
            first: column(u64::MAX, scrolling::Width::Default, row(W, 1.), vec![row(V, 2.)]),
            rest: vec![
                column(3, scrolling::Width::Proportion(0.4), row(V, 1.), vec![]),
                column(4, scrolling::Width::Fixed(700.), row(W, 1.), vec![]),
            ],
        };
        assert_eq!(decode_strip(&encode_strip(strip.clone())), Ok(strip.clone()));
        let placed = strip.place();
        assert_eq!(decode_placed(&encode_placed(placed.clone())), Ok(placed));
        let mut garbled = encode_strip(strip);
        garbled[0] = 0;
        assert_eq!(
            decode_strip(&garbled),
            Err(WireError::Unknown { at: 0, word: 0 })
        );
    }
}
