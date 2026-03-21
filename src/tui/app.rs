use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use crate::models::{Agent, LoadedSession, Session};
use crate::providers::SessionProvider;
use crate::upload::Uploader;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Picker,
    Conversation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentFilter {
    All,
    OnlyClaude,
    OnlyCodex,
}

impl AgentFilter {
    pub fn label(&self) -> &'static str {
        match self {
            AgentFilter::All => "ALL",
            AgentFilter::OnlyClaude => "CLAUDE",
            AgentFilter::OnlyCodex => "CODEX",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            AgentFilter::All => AgentFilter::OnlyClaude,
            AgentFilter::OnlyClaude => AgentFilter::OnlyCodex,
            AgentFilter::OnlyCodex => AgentFilter::All,
        }
    }

    pub fn matches(&self, agent: &Agent) -> bool {
        match self {
            AgentFilter::All => true,
            AgentFilter::OnlyClaude => *agent == Agent::Claude,
            AgentFilter::OnlyCodex => *agent == Agent::Codex,
        }
    }
}

#[derive(Debug)]
pub enum Msg {
    MoveUp,
    MoveDown,
    ScrollUp,
    ScrollDown,
    PageUp,
    PageDown,
    Select,
    Back,
    Quit,
    SearchInput(char),
    SearchBackspace,
    CycleAgentFilter,
    ConfirmUpload,
}

pub struct App {
    pub view: View,
    pub should_quit: bool,

    // Picker state
    pub all_sessions: Vec<Session>,
    pub filtered_indices: Vec<usize>,
    pub selected_index: usize,
    pub search_query: String,
    pub agent_filter: AgentFilter,
    pub matcher: Matcher,

    // Conversation state
    pub loaded_session: Option<LoadedSession>,
    pub scroll_offset: u16,
    pub upload_status: Option<String>,

    // Dependencies
    pub providers: Vec<Box<dyn SessionProvider>>,
    pub uploader: Box<dyn Uploader>,
}

impl App {
    pub fn new(
        sessions: Vec<Session>,
        providers: Vec<Box<dyn SessionProvider>>,
        uploader: Box<dyn Uploader>,
    ) -> Self {
        let filtered_indices: Vec<usize> = (0..sessions.len()).collect();
        Self {
            view: View::Picker,
            should_quit: false,
            all_sessions: sessions,
            filtered_indices,
            selected_index: 0,
            search_query: String::new(),
            agent_filter: AgentFilter::All,
            matcher: Matcher::new(Config::DEFAULT),
            loaded_session: None,
            scroll_offset: 0,
            upload_status: None,
            providers,
            uploader,
        }
    }

    pub fn update(&mut self, msg: Msg) {
        match self.view {
            View::Picker => self.update_picker(msg),
            View::Conversation => self.update_conversation(msg),
        }
    }

    fn update_picker(&mut self, msg: Msg) {
        match msg {
            Msg::MoveUp | Msg::ScrollUp => {
                if self.selected_index > 0 {
                    self.selected_index -= 1;
                }
            }
            Msg::MoveDown | Msg::ScrollDown => {
                if !self.filtered_indices.is_empty()
                    && self.selected_index < self.filtered_indices.len() - 1
                {
                    self.selected_index += 1;
                }
            }
            Msg::PageUp => {
                self.selected_index = self.selected_index.saturating_sub(10);
            }
            Msg::PageDown => {
                if !self.filtered_indices.is_empty() {
                    self.selected_index = (self.selected_index + 10)
                        .min(self.filtered_indices.len() - 1);
                }
            }
            Msg::Select => {
                self.select_session();
            }
            Msg::Quit => {
                if self.search_query.is_empty() {
                    self.should_quit = true;
                } else {
                    self.search_query.clear();
                    self.refilter();
                }
            }
            Msg::Back => {
                if !self.search_query.is_empty() {
                    self.search_query.clear();
                    self.refilter();
                } else {
                    self.should_quit = true;
                }
            }
            Msg::SearchInput(c) => {
                self.search_query.push(c);
                self.refilter();
            }
            Msg::SearchBackspace => {
                self.search_query.pop();
                self.refilter();
            }
            Msg::CycleAgentFilter => {
                self.agent_filter = self.agent_filter.next();
                self.refilter();
            }
            _ => {}
        }
    }

    fn update_conversation(&mut self, msg: Msg) {
        match msg {
            Msg::ScrollUp | Msg::MoveUp => {
                self.scroll_offset = self.scroll_offset.saturating_sub(3);
            }
            Msg::ScrollDown | Msg::MoveDown => {
                self.scroll_offset = self.scroll_offset.saturating_add(3);
            }
            Msg::PageUp => {
                self.scroll_offset = self.scroll_offset.saturating_sub(20);
            }
            Msg::PageDown => {
                self.scroll_offset = self.scroll_offset.saturating_add(20);
            }
            Msg::Back => {
                self.view = View::Picker;
                self.loaded_session = None;
                self.scroll_offset = 0;
                self.upload_status = None;
            }
            Msg::Quit => {
                self.should_quit = true;
            }
            Msg::ConfirmUpload => {
                if let Some(ref loaded) = self.loaded_session {
                    match self.uploader.upload(loaded) {
                        Ok(()) => {
                            self.upload_status = Some("UPLOADED".to_string());
                        }
                        Err(e) => {
                            self.upload_status = Some(format!("ERROR: {}", e));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn select_session(&mut self) {
        if self.filtered_indices.is_empty() {
            return;
        }
        let idx = self.filtered_indices[self.selected_index];
        let session = &self.all_sessions[idx];

        // Try loading messages from providers
        for p in &self.providers {
            if let Ok(messages) = p.load_messages(session) {
                self.loaded_session = Some(LoadedSession {
                    session: session.clone(),
                    messages,
                });
                self.view = View::Conversation;
                self.scroll_offset = 0;
                self.upload_status = None;
                return;
            }
        }
    }

    fn refilter(&mut self) {
        let query = self.search_query.trim();

        if query.is_empty() {
            self.filtered_indices = (0..self.all_sessions.len())
                .filter(|&i| self.agent_filter.matches(&self.all_sessions[i].agent))
                .collect();
        } else {
            let pattern = Pattern::new(
                query,
                CaseMatching::Ignore,
                Normalization::Smart,
                AtomKind::Fuzzy,
            );
            let mut buf = Vec::new();
            self.filtered_indices = (0..self.all_sessions.len())
                .filter(|&i| {
                    let s = &self.all_sessions[i];
                    if !self.agent_filter.matches(&s.agent) {
                        return false;
                    }
                    let haystack = format!(
                        "{} {} {}",
                        s.agent,
                        s.project_path.as_deref().unwrap_or(""),
                        s.first_user_message.as_deref().unwrap_or(""),
                    );
                    let haystack_utf32 = Utf32Str::new(&haystack, &mut buf);
                    pattern
                        .score(haystack_utf32, &mut self.matcher)
                        .is_some()
                })
                .collect();
        }

        // Clamp selection
        if self.filtered_indices.is_empty() {
            self.selected_index = 0;
        } else if self.selected_index >= self.filtered_indices.len() {
            self.selected_index = self.filtered_indices.len() - 1;
        }
    }
}
