use bks_pty::{PtySession, PtySize};

use crate::command_blocks::CommandBlocks;
use crate::profile::Profile;
use crate::terminal_output::{TerminalEvent, TerminalOutput};

pub type SessionId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Starting,
    Running,
    Exited,
    Failed,
    Closing,
}

pub struct TerminalSession {
    pub id: SessionId,
    pub profile_id: String,
    pub title: String,
    pub state: SessionState,
    pub buffer: String,
    pub draft: String,
    pub selection: (u32, u32),
    pub first_visible: usize,
    pub direct_input: bool,
    pub size: PtySize,
    pty: PtySession,
    output: TerminalOutput,
    blocks: CommandBlocks,
}

impl TerminalSession {
    pub fn spawn(id: SessionId, profile: &Profile, size: PtySize) -> std::io::Result<Self> {
        let command = profile.command_line();
        let pty = PtySession::spawn(&command, size)?;
        Ok(Self {
            id,
            profile_id: profile.id.clone(),
            title: profile.name.clone(),
            state: SessionState::Starting,
            buffer: String::new(),
            draft: String::new(),
            selection: (0, 0),
            first_visible: 0,
            direct_input: false,
            size,
            pty,
            output: TerminalOutput::new(),
            blocks: CommandBlocks::new(),
        })
    }

    pub fn poll(&mut self) -> std::io::Result<Option<String>> {
        let bytes = self.pty.drain_output();
        if bytes.is_empty() {
            self.update_state();
            return Ok(None);
        }
        let base = self.buffer.encode_utf16().count() as u32;
        let chunk = self.output.push_chunk(&bytes);
        let text = chunk.text;
        self.buffer.push_str(&text);
        for event in chunk.events {
            match event {
                TerminalEvent::PromptStart(position) => {
                    self.blocks.prompt_started(base + position as u32);
                }
                TerminalEvent::CommandStart(position) => {
                    self.blocks.command_started(base + position as u32);
                }
            }
        }
        if self.blocks.needs_output_start() {
            self.blocks.text_updated(&self.buffer);
        }
        self.update_state();
        if text.is_empty() {
            Ok(None)
        } else {
            Ok(Some(text))
        }
    }

    pub fn write(&self, data: &[u8]) -> std::io::Result<()> {
        self.pty.write_input(data)
    }

    pub fn resize(&mut self, size: PtySize) -> std::io::Result<()> {
        self.pty.resize(size)?;
        self.size = size;
        Ok(())
    }

    pub fn is_alive(&self) -> bool {
        self.pty.is_alive()
    }

    pub fn blocks(&self) -> &CommandBlocks {
        &self.blocks
    }

    pub fn blocks_mut(&mut self) -> &mut CommandBlocks {
        &mut self.blocks
    }

    fn update_state(&mut self) {
        let alive = self.pty.is_alive();
        if alive && self.state == SessionState::Starting {
            self.state = SessionState::Running;
        } else if !alive
            && self.state != SessionState::Failed
            && self.state != SessionState::Closing
        {
            self.state = SessionState::Exited;
        }
    }
}

pub struct SessionManager {
    pub sessions: Vec<TerminalSession>,
    pub active_id: Option<SessionId>,
    next_id: SessionId,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: Vec::new(),
            active_id: None,
            next_id: 1,
        }
    }

    pub fn create(&mut self, profile: &Profile, size: PtySize) -> std::io::Result<SessionId> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        let session = TerminalSession::spawn(id, profile, size)?;
        self.sessions.push(session);
        self.active_id = Some(id);
        Ok(id)
    }

    pub fn activate(&mut self, id: SessionId) -> bool {
        if self.sessions.iter().any(|session| session.id == id) {
            self.active_id = Some(id);
            true
        } else {
            false
        }
    }

    pub fn active(&self) -> Option<&TerminalSession> {
        self.active_id.and_then(|id| self.get(id))
    }

    pub fn active_mut(&mut self) -> Option<&mut TerminalSession> {
        let id = self.active_id?;
        self.get_mut(id)
    }

    pub fn get(&self, id: SessionId) -> Option<&TerminalSession> {
        self.sessions.iter().find(|session| session.id == id)
    }

    pub fn get_mut(&mut self, id: SessionId) -> Option<&mut TerminalSession> {
        self.sessions.iter_mut().find(|session| session.id == id)
    }

    pub fn poll(&mut self) -> Vec<(SessionId, String)> {
        let mut output = Vec::new();
        for session in &mut self.sessions {
            match session.poll() {
                Ok(Some(text)) => output.push((session.id, text)),
                Ok(None) => {}
                Err(_) => session.state = SessionState::Failed,
            }
        }
        output
    }

    pub fn close(&mut self, id: SessionId) -> bool {
        let Some(position) = self.sessions.iter().position(|session| session.id == id) else {
            return false;
        };
        self.sessions.remove(position);
        if self.active_id == Some(id) {
            self.active_id = if self.sessions.is_empty() {
                None
            } else {
                let next = position.min(self.sessions.len() - 1);
                Some(self.sessions[next].id)
            };
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::ProfileKind;
    use std::time::{Duration, Instant};

    fn test_profile(marker: &str) -> Profile {
        Profile {
            id: format!("test-{marker}"),
            name: format!("test-{marker}"),
            kind: ProfileKind::Local,
            executable: "cmd.exe".into(),
            args: vec!["/c".into(), format!("echo BKS_SESSION_{marker}")],
            working_directory: None,
        }
    }

    fn wait_for_markers(
        manager: &mut SessionManager,
        ids: &[SessionId],
        markers: &[&str],
    ) -> Vec<String> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut buffers = vec![String::new(); ids.len()];
        while Instant::now() < deadline {
            for (session_id, text) in manager.poll() {
                if let Some(index) = ids.iter().position(|id| *id == session_id) {
                    buffers[index].push_str(&text);
                }
            }
            if markers
                .iter()
                .enumerate()
                .all(|(index, marker)| buffers[index].contains(marker))
            {
                return buffers;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        buffers
    }

    #[test]
    fn poll_collects_marker() {
        let mut manager = SessionManager::new();
        let id = manager
            .create(&test_profile("ONE"), PtySize::default())
            .unwrap();
        let buffers = wait_for_markers(&mut manager, &[id], &["BKS_SESSION_ONE"]);
        assert!(buffers[0].contains("BKS_SESSION_ONE"));
    }

    #[test]
    fn two_sessions_do_not_mix_buffers() {
        let mut manager = SessionManager::new();
        let a = manager
            .create(&test_profile("A"), PtySize::default())
            .unwrap();
        let b = manager
            .create(&test_profile("B"), PtySize::default())
            .unwrap();
        let buffers = wait_for_markers(&mut manager, &[a, b], &["BKS_SESSION_A", "BKS_SESSION_B"]);
        assert!(buffers[0].contains("BKS_SESSION_A"));
        assert!(!buffers[0].contains("BKS_SESSION_B"));
        assert!(buffers[1].contains("BKS_SESSION_B"));
        assert!(!buffers[1].contains("BKS_SESSION_A"));
    }

    #[test]
    fn activate_preserves_buffer() {
        let mut manager = SessionManager::new();
        let a = manager
            .create(&test_profile("ACT_A"), PtySize::default())
            .unwrap();
        let b = manager
            .create(&test_profile("ACT_B"), PtySize::default())
            .unwrap();
        let _ = wait_for_markers(
            &mut manager,
            &[a, b],
            &["BKS_SESSION_ACT_A", "BKS_SESSION_ACT_B"],
        );
        let a_buffer = manager.get(a).unwrap().buffer.clone();
        assert!(manager.activate(a));
        assert_eq!(manager.active_id, Some(a));
        assert_eq!(manager.get(a).unwrap().buffer, a_buffer);
        assert!(manager.activate(b));
        assert_eq!(manager.active_id, Some(b));
    }

    #[test]
    fn close_selects_another_session() {
        let mut manager = SessionManager::new();
        let a = manager
            .create(&test_profile("CLOSE_A"), PtySize::default())
            .unwrap();
        let b = manager
            .create(&test_profile("CLOSE_B"), PtySize::default())
            .unwrap();
        assert_eq!(manager.active_id, Some(b));
        assert!(manager.close(b));
        assert_eq!(manager.active_id, Some(a));
        assert!(manager.get(a).is_some());
        assert!(manager.get(b).is_none());
        assert!(manager.close(a));
        assert_eq!(manager.active_id, None);
    }
}
