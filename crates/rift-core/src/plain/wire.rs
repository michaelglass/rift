//! The core's events and effects on plain data, as `u64` words: the Roc core's boundary (one
//! event in, one effect list out) and the recorded-trace format (event, effect list, …).
//!
//! Every field is one word: integers zero-extended (a pid sign-extended), a float as its bits,
//! an enum as its variant's index. An `Option` is a flag word (0 none, 1 some) and then the
//! value; a list is its length and then its items. Each event and effect starts with its tag,
//! the index in the tables below, which `roc/core` mirrors.

use crate::core::{Desire, Effect, Event, Plan};
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
    let mut words = Words::default();
    words.event(event);
    words.0
}

/// `effects` as words.
pub fn encode_effects(effects: Vec<Effect<Plain>>) -> Vec<u64> {
    let mut words = Words::default();
    words.list(effects, Words::effect);
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
}
