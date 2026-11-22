//! Session state machine (spec 03, session state).

use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum State {
    Idle,
    Searching,
    Armed,
    Playing,
    Results,
    Paused,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Started,
    /// Layout found and our side known.
    LayoutReady,
    /// First head tracked moving toward our receptors.
    HeadApproaching,
    /// Receptors missing for 1.5 s, or the results panel matched.
    SongOver,
    HistoryWritten,
    TogglePause,
    FocusLost,
    FocusGained,
}

#[derive(Debug)]
pub struct Session {
    state: State,
    /// State to return to from `Paused`, and whether focus (not the hotkey) paused it.
    resume: Option<(State, bool)>,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            state: State::Idle,
            resume: None,
        }
    }
}

impl Session {
    pub fn state(&self) -> State {
        self.state
    }

    /// Applies `ev`; returns the new state if it changed.
    pub fn on(&mut self, ev: Event) -> Option<State> {
        use {Event as E, State as S};
        let next = match (self.state, ev) {
            (S::Paused, E::TogglePause) | (S::Paused, E::FocusGained)
                if ev == E::TogglePause || self.resume.is_some_and(|r| r.1) =>
            {
                self.resume.take().map_or(S::Searching, |r| r.0)
            }
            (S::Paused, _) => return None,
            (s, E::TogglePause) => {
                self.resume = Some((s, false));
                S::Paused
            }
            (s, E::FocusLost) => {
                self.resume = Some((s, true));
                S::Paused
            }
            (S::Idle, E::Started) => S::Searching,
            (S::Searching, E::LayoutReady) => S::Armed,
            (S::Armed, E::HeadApproaching) => S::Playing,
            (S::Playing, E::SongOver) => S::Results,
            (S::Results, E::HistoryWritten) => S::Searching,
            _ => return None,
        };
        (next != self.state).then(|| {
            self.state = next;
            next
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Event as E, State as S, *};

    #[test]
    fn song_cycle() {
        let mut s = Session::default();
        for (ev, want) in [
            (E::Started, S::Searching),
            (E::LayoutReady, S::Armed),
            (E::HeadApproaching, S::Playing),
            (E::SongOver, S::Results),
            (E::HistoryWritten, S::Searching),
        ] {
            assert_eq!(s.on(ev), Some(want));
        }
        assert_eq!(s.on(E::HeadApproaching), None);
    }

    #[test]
    fn pause_returns_to_previous() {
        let mut s = Session::default();
        s.on(E::Started);
        s.on(E::LayoutReady);
        assert_eq!(s.on(E::FocusLost), Some(S::Paused));
        assert_eq!(s.on(E::HeadApproaching), None);
        assert_eq!(s.on(E::FocusGained), Some(S::Armed));
        // Hotkey pause is not undone by focus.
        s.on(E::TogglePause);
        assert_eq!(s.on(E::FocusGained), None);
        assert_eq!(s.on(E::TogglePause), Some(S::Armed));
    }
}
