use std::{
    path::PathBuf,
    str::FromStr,
    sync::{Arc, RwLock},
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use flume::Sender;
use tokio::task::JoinHandle;
use tui::{
    layout::{Alignment, Rect},
    style::{Color, Style},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};
use urlencoding::encode;
use ytpapi::{Playlist, Video, YTApi};

use crate::{
    consts::CONFIG, run_service, structures::sound_action::SoundAction, systems::logger::log_,
    tasks, utils::invert, DATABASE,
};

use super::{
    item_list::{ListItem, ListItemAction},
    playlist::format_playlist,
    split_y_start, EventResponse, ManagerMessage, Screen, Screens,
};

pub struct Search {
    pub text: String,
    pub goto: Screens,
    pub list: Arc<RwLock<ListItem<Status>>>,
    pub search_handle: Option<JoinHandle<()>>,
    pub api: Option<Arc<ytpapi::YTApi>>,
    pub action_sender: Arc<Sender<SoundAction>>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Local(Video),
    Unknown(Video),
    PlayList(Playlist, Vec<Video>),
}
impl ListItemAction for Status {
    fn render_style(&self, _: &str, selected: bool) -> Style {
        let k = match self {
            Self::Local(_) => CONFIG.player.text_next_style,
            Self::Unknown(_) => CONFIG.player.text_downloading_style,
            Self::PlayList(_, _) => CONFIG.player.text_next_style,
        };
        if selected {
            invert(k)
        } else {
            k
        }
    }
}

impl Screen for Search {
    fn on_mouse_press(
        &mut self,
        mouse_event: crossterm::event::MouseEvent,
        frame_data: &Rect,
    ) -> EventResponse {
        let splitted = split_y_start(*frame_data, 3);
        if let Some(e) = self
            .list
            .write()
            .unwrap()
            .on_mouse_press(mouse_event, &splitted[1])
        {
            self.execute_status(e, mouse_event.modifiers)
        } else {
            EventResponse::None
        }
    }

    fn on_key_press(&mut self, key: KeyEvent, _: &Rect) -> EventResponse {
        if KeyCode::Esc == key.code {
            return ManagerMessage::ChangeState(self.goto).event();
        }
        if let Some(e) = self.list.write().unwrap().on_key_press(key) {
            return self.execute_status(e.clone(), key.modifiers);
        }
        let textbefore = self.text.trim().to_owned();
        match key.code {
            KeyCode::Delete | KeyCode::Backspace => {
                self.text.pop();
            }
            KeyCode::Char(a) => {
                self.text.push(a);
            }
            _ => {}
        }
        if textbefore == self.text.trim() {
            return EventResponse::None;
        }

        if let Some(handle) = self.search_handle.take() {
            handle.abort();
        }

        let text = self.text.to_lowercase();

        let local = DATABASE
            .read()
            .unwrap()
            .iter()
            .filter(|x| {
                x.title.to_lowercase().contains(&text) || x.author.to_lowercase().contains(&text)
            })
            .cloned()
            .map(|video| (format!(" {video} "), Status::Local(video)))
            .take(100)
            .collect::<Vec<_>>();
        self.list.write().unwrap().update_contents(local.clone());

        if let Some(api) = self.api.clone() {
            let text = self.text.clone();
            let items = self.list.clone();
            self.search_handle = Some(run_service(async move {
                // Sleep to prevent spamming the api
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                let mut item = Vec::new();
                match api.search(&encode(&text).replace("%20", "+")).await {
                    Ok((e, p)) => {
                        for video in e.into_iter() {
                            let id = video.video_id.clone();
                            item.push((
                                format!(" {video} "),
                                if DATABASE.read().unwrap().iter().any(|x| x.video_id == id) {
                                    Status::Local(video)
                                } else {
                                    Status::Unknown(video)
                                },
                            ));
                        }
                        for playlist in p.into_iter() {
                            let api = api.clone();
                            let items = items.clone();
                            run_service(async move {
                                match api.browse_playlist(&playlist.browse_id).await {
                                    Ok(e) => {
                                        if e.is_empty() {
                                            return;
                                        }
                                        items.write().unwrap().add_element((
                                            format_playlist(
                                                &format!(
                                                    " [P] {} ({})",
                                                    playlist.name, playlist.subtitle
                                                ),
                                                &e,
                                            ),
                                            Status::PlayList(playlist, e),
                                        ));
                                    }
                                    Err(e) => {
                                        log_(format!("{:?}", e));
                                    }
                                };
                            });
                        }
                    }
                    Err(e) => {
                        log_(format!("{:?}", e));
                    }
                }
                let mut local = local;
                local.append(&mut item);
                items.write().unwrap().update_contents(local);
            }));
        }

        EventResponse::None
    }

    fn render(&mut self, frame: &mut Frame<tui::backend::CrosstermBackend<std::io::Stdout>>) {
        let splitted = split_y_start(frame.size(), 3);
        frame.render_widget(
            Paragraph::new(self.text.clone())
                .style(Style::default().fg(Color::LightCyan))
                .alignment(Alignment::Center)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .style(Style::default().fg(Color::White))
                        .title(" Search ")
                        .border_type(BorderType::Plain),
                ),
            splitted[0],
        );
        //  Select the playlist to play
        let items = self.list.read().unwrap();
        frame.render_widget(&*items, splitted[1]);
    }

    fn handle_global_message(&mut self, _: super::ManagerMessage) -> EventResponse {
        EventResponse::None
    }

    fn close(&mut self, _: Screens) -> EventResponse {
        EventResponse::None
    }

    fn open(&mut self) -> EventResponse {
        EventResponse::None
    }
}
impl Search {
    pub async fn new(action_sender: Arc<Sender<SoundAction>>) -> Self {
        Self {
            text: String::new(),
            list: Arc::new(RwLock::new(ListItem::new(
                "Select a song to play".to_string(),
            ))),
            goto: Screens::MusicPlayer,
            search_handle: None,
            api: YTApi::from_header_file(PathBuf::from_str("headers.txt").unwrap().as_path())
                .await
                .ok()
                .map(Arc::new),
            action_sender,
        }
    }

    pub fn execute_status(&self, e: Status, modifiers: KeyModifiers) -> EventResponse {
        match e {
            Status::Local(e) | Status::Unknown(e) => {
                self.action_sender
                    .send(SoundAction::AddVideoUnary(e.clone()))
                    .unwrap();
                tasks::download::start_task_unary(self.action_sender.clone(), e);
                if modifiers.contains(KeyModifiers::CONTROL) {
                    EventResponse::None
                } else {
                    ManagerMessage::PlayerFrom(Screens::Playlist).event()
                }
            }
            Status::PlayList(e, v) => ManagerMessage::Inspect(e.name, Screens::Search, v)
                .pass_to(Screens::PlaylistViewer)
                .event(),
        }
    }
}
