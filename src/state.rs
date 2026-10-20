#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase { Idle, Selecting, Countdown, Recording, Stopping }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoAction { Start, CancelSelection, CancelCountdown, StopAndSave, Wait }

impl Phase {
    pub fn video_action(self) -> VideoAction {
        match self {Self::Idle=>VideoAction::Start, Self::Selecting=>VideoAction::CancelSelection,
            Self::Countdown=>VideoAction::CancelCountdown,Self::Recording=>VideoAction::StopAndSave, Self::Stopping=>VideoAction::Wait}
    }
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn video_toggle_cancels_before_start_and_saves_after_start() {
        assert_eq!(Phase::Countdown.video_action(),VideoAction::CancelCountdown);
        assert_eq!(Phase::Recording.video_action(),VideoAction::StopAndSave);
        assert_eq!(Phase::Stopping.video_action(),VideoAction::Wait);
    }
}
