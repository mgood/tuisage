//! Execution component — embedded terminal for running commands.
//!
//! This component owns the full lifecycle of a command execution:
//! PTY I/O, VT100 rendering, keyboard forwarding, and close-on-exit.

use std::io::Read;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use portable_pty::{CommandBuilder, MasterPty, NativePtySystem, PtySize, PtySystem};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect, Size},
    style::{Modifier as RatModifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget, Wrap},
};
use tui_term::widget::PseudoTerminal;

use super::{Component, EventResult, RenderableComponent};
use crate::theme::UiColors;

/// Actions the execution component can emit to its parent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionAction {
    /// User closed the execution view (process had exited).
    Close,
}

/// State for a running (or finished) command execution.
pub struct ExecutionState {
    /// The command string displayed at the top.
    pub command_display: String,
    /// vt100 parser holding the terminal screen state.
    pub parser: Arc<RwLock<vt100::Parser>>,
    /// Writer to send input to the PTY (None after the master is dropped).
    pub pty_writer: Arc<Mutex<Option<Box<dyn Write + Send>>>>,
    /// PTY master for resizing.
    pub pty_master: Arc<Mutex<Option<Box<dyn MasterPty + Send>>>>,
    /// Whether the child process has exited.
    pub exited: Arc<AtomicBool>,
    /// Exit status description (e.g. "0", "1", "signal 9").
    pub exit_status: Arc<Mutex<Option<String>>>,
}

/// Embedded terminal component for command execution.
pub struct ExecutionComponent {
    state: ExecutionState,
}

impl ExecutionComponent {
    pub fn new(state: ExecutionState) -> Self {
        Self { state }
    }

    pub fn spawn(
        command_display: String,
        parts: &[String],
        terminal_size: Size,
    ) -> color_eyre::Result<Self> {
        if parts.is_empty() {
            return Err(color_eyre::eyre::eyre!("No command to execute"));
        }

        let mut cmd = CommandBuilder::new(&parts[0]);
        for arg in &parts[1..] {
            cmd.arg(arg);
        }
        if let Ok(cwd) = std::env::current_dir() {
            cmd.cwd(cwd);
        }

        let pty_size = Self::pty_size_for_terminal(terminal_size);
        let pty_system = NativePtySystem::default();
        let pair = pty_system
            .openpty(pty_size)
            .map_err(|e| color_eyre::eyre::eyre!("Failed to open PTY: {}", e))?;

        let parser = Arc::new(RwLock::new(vt100::Parser::new(
            pty_size.rows,
            pty_size.cols,
            0,
        )));
        let exited = Arc::new(AtomicBool::new(false));
        let exit_status: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

        let child_result = pair.slave.spawn_command(cmd);
        drop(pair.slave);

        let mut child = child_result.map_err(|e| {
            color_eyre::eyre::eyre!("Failed to spawn command '{}': {}", parts[0], e)
        })?;

        {
            let exited = exited.clone();
            let exit_status = exit_status.clone();
            std::thread::spawn(move || {
                match child.wait() {
                    Ok(status) => {
                        if let Ok(mut s) = exit_status.lock() {
                            *s = Some(format!("{}", status));
                        }
                    }
                    Err(e) => {
                        if let Ok(mut s) = exit_status.lock() {
                            *s = Some(format!("error: {}", e));
                        }
                    }
                }
                exited.store(true, Ordering::Relaxed);
            });
        }

        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| color_eyre::eyre::eyre!("Failed to clone PTY reader: {}", e))?;

        {
            let parser = parser.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 8192];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(size) => {
                            if let Ok(mut p) = parser.write() {
                                p.process(&buf[..size]);
                            }
                        }
                        Err(_) => break,
                    }
                }
            });
        }

        let writer = pair
            .master
            .take_writer()
            .map_err(|e| color_eyre::eyre::eyre!("Failed to take PTY writer: {}", e))?;

        let pty_writer: Arc<Mutex<Option<Box<dyn Write + Send>>>> =
            Arc::new(Mutex::new(Some(writer)));
        let pty_master: Arc<Mutex<Option<Box<dyn MasterPty + Send>>>> =
            Arc::new(Mutex::new(Some(pair.master)));

        {
            let exited = exited.clone();
            let pty_writer = pty_writer.clone();
            let pty_master = pty_master.clone();
            std::thread::spawn(move || {
                while !exited.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(50));
                }
                std::thread::sleep(Duration::from_millis(100));
                if let Ok(mut w) = pty_writer.lock() {
                    *w = None;
                }
                if let Ok(mut m) = pty_master.lock() {
                    *m = None;
                }
            });
        }

        Ok(Self::new(ExecutionState {
            command_display,
            parser,
            pty_writer,
            pty_master,
            exited,
            exit_status,
        }))
    }

    pub fn resize_to_terminal(&self, terminal_size: Size) {
        let pty_size = Self::pty_size_for_terminal(terminal_size);
        self.resize_pty(pty_size.rows, pty_size.cols);
    }

    fn pty_size_for_terminal(terminal_size: Size) -> PtySize {
        PtySize {
            rows: terminal_size.height.saturating_sub(4).max(4),
            cols: terminal_size.width.max(20),
            pixel_width: 0,
            pixel_height: 0,
        }
    }

    /// Whether the child process has exited.
    pub fn exited(&self) -> bool {
        self.state.exited.load(Ordering::Relaxed)
    }

    /// Get the exit status string, if available.
    pub fn exit_status(&self) -> Option<String> {
        self.state
            .exit_status
            .lock()
            .ok()
            .and_then(|s| s.clone())
    }

    /// Write bytes to the PTY (forward keyboard input).
    pub fn write_to_pty(&self, data: &[u8]) {
        if let Ok(mut writer_guard) = self.state.pty_writer.lock() {
            if let Some(ref mut writer) = *writer_guard {
                let _ = writer.write_all(data);
                let _ = writer.flush();
            }
        }
    }

    /// Resize the PTY to fit new terminal dimensions.
    pub fn resize_pty(&self, rows: u16, cols: u16) {
        if let Ok(mut master_guard) = self.state.pty_master.lock() {
            if let Some(ref mut master) = *master_guard {
                let _ = master.resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                });
            }
        }
        if let Ok(mut parser_guard) = self.state.parser.write() {
            parser_guard.screen_mut().set_size(rows, cols);
        }
    }

    /// Forward a key event to the PTY as raw bytes.
    fn forward_key_to_pty(&self, key: KeyEvent) {
        let bytes: Option<Vec<u8>> = match key.code {
            KeyCode::Char(c) => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    match c {
                        'c' => return self.write_to_pty(b"\x03"),
                        'd' => return self.write_to_pty(b"\x04"),
                        _ => {}
                    }
                }
                let mut buf = [0u8; 4];
                let s = c.encode_utf8(&mut buf);
                Some(s.as_bytes().to_vec())
            }
            KeyCode::Enter => Some(b"\r".to_vec()),
            KeyCode::Backspace => Some(b"\x7f".to_vec()),
            KeyCode::Tab => Some(b"\t".to_vec()),
            KeyCode::Esc => Some(b"\x1b".to_vec()),
            KeyCode::Up => Some(b"\x1b[A".to_vec()),
            KeyCode::Down => Some(b"\x1b[B".to_vec()),
            KeyCode::Right => Some(b"\x1b[C".to_vec()),
            KeyCode::Left => Some(b"\x1b[D".to_vec()),
            KeyCode::Home => Some(b"\x1b[H".to_vec()),
            KeyCode::End => Some(b"\x1b[F".to_vec()),
            KeyCode::Delete => Some(b"\x1b[3~".to_vec()),
            _ => None,
        };

        if let Some(data) = bytes {
            self.write_to_pty(&data);
        }
    }
}

impl Component for ExecutionComponent {
    type Action = ExecutionAction;

    fn handle_key(&mut self, key: KeyEvent) -> EventResult<ExecutionAction> {
        if self.exited() {
            match key.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                    EventResult::Action(ExecutionAction::Close)
                }
                _ => EventResult::Consumed,
            }
        } else {
            self.forward_key_to_pty(key);
            EventResult::Consumed
        }
    }

    fn handle_mouse(&mut self, _event: MouseEvent, _area: Rect) -> EventResult<ExecutionAction> {
        EventResult::NotHandled
    }
}

impl RenderableComponent for ExecutionComponent {
    fn render(&mut self, area: Rect, buf: &mut Buffer, colors: &UiColors) {
        let outer = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // command display
                Constraint::Min(4),    // terminal output
                Constraint::Length(1), // status bar
            ])
            .split(area);

        // --- Command display at top ---
        let cmd_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(colors.active_border))
            .title(" Command ")
            .title_style(Style::default().fg(colors.active_border).bold());

        let cmd_spans = vec![
            Span::styled("$ ", Style::default().fg(colors.command)),
            Span::styled(
                self.state.command_display.clone(),
                Style::default()
                    .fg(colors.preview_cmd)
                    .add_modifier(RatModifier::BOLD),
            ),
        ];
        let cmd_paragraph = Paragraph::new(Line::from(cmd_spans))
            .block(cmd_block)
            .wrap(Wrap { trim: false });
        Widget::render(cmd_paragraph, outer[0], buf);

        // --- Terminal output ---
        let term_block = Block::default().borders(Borders::NONE);

        if let Ok(parser) = self.state.parser.read() {
            let pseudo_term = PseudoTerminal::new(parser.screen())
                .block(term_block)
                .style(Style::default().fg(colors.preview_cmd).bg(colors.bg));
            Widget::render(pseudo_term, outer[1], buf);
        } else {
            let fallback = Paragraph::new("(terminal output unavailable)")
                .block(term_block)
                .style(Style::default().fg(colors.help));
            Widget::render(fallback, outer[1], buf);
        }

        // --- Status bar at bottom ---
        let exited = self.exited();
        let status_text = if exited {
            let exit_code = self.exit_status().unwrap_or_default();
            format!(
                " Exited ({}) — press Esc/⏎/q to close ",
                if exit_code.is_empty() {
                    "unknown".to_string()
                } else {
                    exit_code
                }
            )
        } else {
            " Running… (input is forwarded to the process) ".to_string()
        };

        let status_style = if exited {
            Style::default()
                .fg(colors.help)
                .add_modifier(RatModifier::BOLD | RatModifier::REVERSED)
        } else {
            Style::default()
                .fg(colors.command)
                .add_modifier(RatModifier::BOLD | RatModifier::REVERSED)
        };

        let status = Paragraph::new(status_text).style(status_style);
        Widget::render(status, outer[2], buf);
    }
}
