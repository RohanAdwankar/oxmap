use std::cmp::{max, min};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use oxdraw::{
    Diagram as OxDiagram, Edge as OxEdge, EdgeArrowDirection as OxArrowDirection,
    EdgeKind as OxEdgeKind, LayoutOverrides as OxLayoutOverrides, Point as OxPoint,
    edge_identifier,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};
use serde::{Deserialize, Serialize};

fn main() -> Result<()> {
    let args = Args::from_env()?;
    let mut terminal = setup_terminal()?;
    let app_result = run_app(&mut terminal, args);
    restore_terminal(&mut terminal)?;
    app_result
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<std::io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    Ok(Terminal::new(backend)?)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>, args: Args) -> Result<()> {
    let mut app = App::load(args)?;

    loop {
        terminal.draw(|frame| {
            app.last_canvas = content_area(frame.area());
            app.render(frame.area(), frame.buffer_mut());
        })?;

        if app.should_quit {
            break;
        }

        if !event::poll(Duration::from_millis(50))? {
            continue;
        }

        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => app.handle_key(key, terminal)?,
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollUp => app.zoom_by(1.2),
                MouseEventKind::ScrollDown => app.zoom_by(1.0 / 1.2),
                _ => {}
            },
            _ => {}
        }
    }

    Ok(())
}

#[derive(Default)]
struct Args {
    path: Option<PathBuf>,
}

impl Args {
    fn from_env() -> Result<Self> {
        let mut args = env::args().skip(1);
        let path = args.next().map(PathBuf::from);
        if let Some(extra) = args.next() {
            bail!("unexpected extra argument: {extra}");
        }
        Ok(Self { path })
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Note {
    key: char,
    title: String,
    body: String,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: NoteColor,
    file_path: Option<PathBuf>,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
enum NoteColor {
    Cyan,
    Yellow,
    Green,
    Magenta,
    Blue,
    Red,
    White,
}

impl NoteColor {
    fn ratatui(self) -> Color {
        match self {
            Self::Cyan => Color::Indexed(116),
            Self::Yellow => Color::Indexed(221),
            Self::Green => Color::Indexed(114),
            Self::Magenta => Color::Indexed(176),
            Self::Blue => Color::Indexed(75),
            Self::Red => Color::Indexed(210),
            Self::White => Color::Indexed(231),
        }
    }

    fn cycle(index: usize) -> Self {
        match index % 7 {
            0 => Self::Cyan,
            1 => Self::Yellow,
            2 => Self::Green,
            3 => Self::Magenta,
            4 => Self::Blue,
            5 => Self::Red,
            _ => Self::White,
        }
    }
}

#[derive(Clone, Copy, Serialize, Deserialize)]
enum RelationType {
    Directional,
    Bidirectional,
    Compositional,
    Cluster,
}

impl RelationType {
    fn from_digit(digit: u16) -> Option<Self> {
        match digit {
            1 => Some(Self::Directional),
            2 => Some(Self::Bidirectional),
            3 => Some(Self::Compositional),
            4 => Some(Self::Cluster),
            _ => None,
        }
    }

    fn stroke(self) -> char {
        match self {
            Self::Directional => '─',
            Self::Bidirectional => '═',
            Self::Compositional => '━',
            Self::Cluster => '┈',
        }
    }

    fn tip(self, from_start: bool, dx: i32, dy: i32) -> char {
        match self {
            Self::Directional => {
                if from_start {
                    ' '
                } else {
                    arrow_glyph(dx, dy)
                }
            }
            Self::Bidirectional => arrow_glyph(dx, dy),
            Self::Compositional => {
                if from_start {
                    '◆'
                } else {
                    arrow_glyph(dx, dy)
                }
            }
            Self::Cluster => '◌',
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Directional => "directional",
            Self::Bidirectional => "bidirectional",
            Self::Compositional => "compositional",
            Self::Cluster => "cluster",
        }
    }

    fn mermaid_operator(self) -> &'static str {
        match self {
            Self::Directional => "-->",
            Self::Bidirectional => "<-->",
            Self::Compositional => "--o",
            Self::Cluster => "-->",
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Relation {
    from: char,
    to: char,
    kind: RelationType,
}

#[derive(Serialize, Deserialize)]
struct GraphFile {
    notes: Vec<Note>,
    relations: Vec<Relation>,
    editor_command: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct SavedLayoutOverrides {
    #[serde(default)]
    nodes: HashMap<String, SavedPoint>,
}

#[derive(Clone, Copy, Default, Serialize, Deserialize)]
struct SavedPoint {
    x: f32,
    y: f32,
}

#[derive(Clone, Copy, Deserialize)]
struct OxPointView {
    x: f32,
    y: f32,
}

#[derive(Default, Serialize, Deserialize)]
struct AppConfig {
    #[serde(rename = "editor", alias = "editor_command")]
    editor_command: Option<String>,
    monocolor: Option<bool>,
    #[serde(rename = "movement_step", alias = "movement_speed")]
    movement_speed: Option<f32>,
}

enum Mode {
    Normal,
    AwaitSelect,
    AwaitRelationPrefix(RelationType),
    AwaitRelationTarget(RelationType),
    AwaitUnlinkPrefix,
    AwaitUnlinkTarget,
    Edit(EditState),
    Command,
}

struct EditState {
    field: EditField,
    title_cursor: usize,
    body_cursor: usize,
}

#[derive(Clone, Copy)]
enum EditField {
    Title,
    Body,
}

struct App {
    notes: Vec<Note>,
    relations: Vec<Relation>,
    selected: Option<usize>,
    mode: Mode,
    camera_x: f32,
    camera_y: f32,
    zoom: f32,
    count_buffer: String,
    command_buffer: String,
    status: String,
    editor_command: String,
    monocolor: bool,
    movement_speed: f32,
    loaded_path: Option<PathBuf>,
    dirty: bool,
    should_quit: bool,
    last_canvas: Rect,
}

impl App {
    fn load(args: Args) -> Result<Self> {
        let mut app = Self::demo();
        app.load_config()?;
        app.loaded_path = args.path.clone();
        if let Some(path) = args.path {
            app.load_from_path(&path)?;
            app.status = format!("loaded {}", path.display());
            app.fit_all();
        }
        Ok(app)
    }

    fn demo() -> Self {
        Self {
            notes: vec![Note {
                key: 'a',
                title: String::new(),
                body: String::new(),
                x: -10.0,
                y: -4.0,
                w: 20.0,
                h: 8.0,
                color: NoteColor::Cyan,
                file_path: Some(PathBuf::from("nodes/a.md")),
            }],
            relations: vec![],
            selected: Some(0),
            mode: Mode::Edit(EditState {
                field: EditField::Title,
                title_cursor: 0,
                body_cursor: 0,
            }),
            camera_x: 0.0,
            camera_y: 0.0,
            zoom: 1.4,
            count_buffer: String::new(),
            command_buffer: String::new(),
            status: "start typing".into(),
            editor_command: env::var("EDITOR").unwrap_or_else(|_| "vi".into()),
            monocolor: false,
            movement_speed: 2.0,
            loaded_path: None,
            dirty: false,
            should_quit: false,
            last_canvas: Rect::default(),
        }
    }

    fn load_from_path(&mut self, path: &Path) -> Result<()> {
        match path.extension().and_then(|ext| ext.to_str()) {
            Some("json") => {
                let data = fs::read_to_string(path)
                    .with_context(|| format!("failed to read {}", path.display()))?;
                let graph: GraphFile =
                    serde_json::from_str(&data).context("failed to parse graph json")?;
                self.notes = graph.notes;
                self.relations = graph.relations;
                if let Some(editor) = graph.editor_command {
                    self.editor_command = editor;
                }
            }
            Some("mmd") => {
                let data = fs::read_to_string(path)
                    .with_context(|| format!("failed to read {}", path.display()))?;
                let graph = parse_saved_mmd(&data)?;
                self.notes = graph.notes;
                self.relations = graph.relations;
            }
            _ => bail!("unsupported file type for {}", path.display()),
        }
        self.selected = None;
        self.dirty = false;
        Ok(())
    }

    fn handle_key(
        &mut self,
        key: KeyEvent,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    ) -> Result<()> {
        if matches!(self.mode, Mode::Command) {
            self.handle_command_key(key)
        } else if matches!(self.mode, Mode::Edit(_)) {
            self.handle_edit_key(key)
        } else {
            self.handle_normal_key(key, terminal)
        }
    }

    fn handle_normal_key(
        &mut self,
        key: KeyEvent,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    ) -> Result<()> {
        if matches!(key.code, KeyCode::Esc) {
            self.cancel_pending();
            return Ok(());
        }

        match &mut self.mode {
            Mode::AwaitSelect => {
                if let KeyCode::Char(ch) = key.code {
                    self.select_by_key(ch);
                }
                return Ok(());
            }
            Mode::AwaitRelationPrefix(kind) => {
                if let KeyCode::Char('f') = key.code {
                    let relation = *kind;
                    self.mode = Mode::AwaitRelationTarget(relation);
                    self.status = format!("relation {}: target key", relation.label());
                } else {
                    self.status = "relation expects f then target key".into();
                    self.mode = Mode::Normal;
                }
                return Ok(());
            }
            Mode::AwaitRelationTarget(kind) => {
                if let KeyCode::Char(ch) = key.code {
                    let relation = *kind;
                    self.create_relation_to(ch, relation);
                }
                return Ok(());
            }
            Mode::AwaitUnlinkPrefix => {
                if let KeyCode::Char('f') = key.code {
                    self.mode = Mode::AwaitUnlinkTarget;
                    self.status = "unlink: target key".into();
                } else {
                    self.status = "unlink expects f then target key".into();
                    self.mode = Mode::Normal;
                }
                return Ok(());
            }
            Mode::AwaitUnlinkTarget => {
                if let KeyCode::Char(ch) = key.code {
                    self.remove_relation_to(ch);
                }
                return Ok(());
            }
            Mode::Normal | Mode::Edit(_) | Mode::Command => {}
        }

        if let KeyCode::Char(ch) = key.code
            && ch.is_ascii_digit()
        {
            self.count_buffer.push(ch);
            self.status = format!("count {}", self.count_buffer);
            return Ok(());
        }

        let count = self.take_count();
        match key.code {
            KeyCode::Char('q') => self.quit_requested(false),
            KeyCode::Char(':') => {
                self.mode = Mode::Command;
                self.command_buffer.clear();
                self.status = "command mode".into();
            }
            KeyCode::Char('a') => self.add_node(),
            KeyCode::Char('x') => self.delete_selected_note(),
            KeyCode::Char('f') => {
                self.mode = Mode::AwaitSelect;
                self.status = "select node by key".into();
            }
            KeyCode::Char('i') => self.start_edit(),
            KeyCode::Char('o') => self.open_selected_in_editor(terminal)?,
            KeyCode::Char('m') => {
                let relation = RelationType::from_digit(count).unwrap_or(RelationType::Directional);
                self.begin_relation(relation);
            }
            KeyCode::Char('u') => self.begin_unlink(),
            KeyCode::Char('G') => self.fit_all(),
            KeyCode::Char('s') | KeyCode::Char('+') | KeyCode::Char('=') => self.zoom_by(1.2),
            KeyCode::Char('d') | KeyCode::Char('-') | KeyCode::Char('_') => self.zoom_by(1.0 / 1.2),
            KeyCode::Left | KeyCode::Char('h') => self.apply_motion(-1.0, 0.0, count),
            KeyCode::Right | KeyCode::Char('l') => self.apply_motion(1.0, 0.0, count),
            KeyCode::Up | KeyCode::Char('k') => self.apply_motion(0.0, -1.0, count),
            KeyCode::Down | KeyCode::Char('j') => self.apply_motion(0.0, 1.0, count),
            _ => {
                self.count_buffer.clear();
            }
        }
        Ok(())
    }

    fn handle_edit_key(&mut self, key: KeyEvent) -> Result<()> {
        let Some(selected) = self.selected else {
            self.mode = Mode::Normal;
            return Ok(());
        };
        let Mode::Edit(state) = &mut self.mode else {
            return Ok(());
        };

        match key.code {
            KeyCode::Esc => {
                let note_key = self.notes[selected].key;
                self.mode = Mode::Normal;
                self.status = format!("edited {}", note_key);
            }
            KeyCode::Tab => {
                state.field = match state.field {
                    EditField::Title => EditField::Body,
                    EditField::Body => EditField::Title,
                };
            }
            KeyCode::Backspace => {
                let note = &mut self.notes[selected];
                let (text, cursor) = match state.field {
                    EditField::Title => (&mut note.title, &mut state.title_cursor),
                    EditField::Body => (&mut note.body, &mut state.body_cursor),
                };
                if *cursor > 0 && *cursor <= text.len() {
                    text.remove(*cursor - 1);
                    *cursor -= 1;
                    self.dirty = true;
                }
            }
            KeyCode::Enter => {
                let note = &mut self.notes[selected];
                let (text, cursor) = match state.field {
                    EditField::Title => (&mut note.title, &mut state.title_cursor),
                    EditField::Body => (&mut note.body, &mut state.body_cursor),
                };
                text.insert(*cursor, '\n');
                *cursor += 1;
                self.dirty = true;
            }
            KeyCode::Left => {
                let len = match state.field {
                    EditField::Title => self.notes[selected].title.len(),
                    EditField::Body => self.notes[selected].body.len(),
                };
                move_cursor(state, -1, len);
            }
            KeyCode::Right => {
                let len = match state.field {
                    EditField::Title => self.notes[selected].title.len(),
                    EditField::Body => self.notes[selected].body.len(),
                };
                move_cursor(state, 1, len);
            }
            KeyCode::Char(ch) => {
                let note = &mut self.notes[selected];
                let (text, cursor) = match state.field {
                    EditField::Title => (&mut note.title, &mut state.title_cursor),
                    EditField::Body => (&mut note.body, &mut state.body_cursor),
                };
                text.insert(*cursor, ch);
                *cursor += ch.len_utf8();
                self.dirty = true;
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_command_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.command_buffer.clear();
                self.status = "command cancelled".into();
            }
            KeyCode::Backspace => {
                self.command_buffer.pop();
            }
            KeyCode::Enter => {
                let command = self.command_buffer.trim().to_string();
                self.command_buffer.clear();
                self.mode = Mode::Normal;
                self.run_command(&command)?;
            }
            KeyCode::Char(ch) => self.command_buffer.push(ch),
            KeyCode::Tab => self.command_buffer.push(' '),
            _ => {}
        }
        Ok(())
    }

    fn run_command(&mut self, command: &str) -> Result<()> {
        match command {
            "w" => self.save_loaded()?,
            "wq" => {
                self.save_loaded()?;
                self.should_quit = true;
            }
            "q" => self.quit_requested(false),
            "q!" => self.quit_requested(true),
            "export" => {
                let path = self.export_mermaid()?;
                self.status = format!("exported {}", path.display());
            }
            _ if command.starts_with("editor") => {
                let value = command["editor".len()..].trim();
                if value.is_empty() {
                    self.status = format!("editor {}", self.editor_command);
                } else {
                    self.editor_command = value.into();
                    self.save_config()?;
                    self.dirty = true;
                    self.status = format!("editor set to {}", self.editor_command);
                }
            }
            "" => {}
            _ => self.status = format!("unknown command: :{command}"),
        }
        Ok(())
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        let canvas = content_area(area);
        if canvas.width < 4 || canvas.height < 4 {
            return;
        }

        fill_area(buf, canvas, ' ', Style::default().bg(Color::Black));
        self.render_relations(canvas, buf);
        self.render_notes(canvas, buf);
        self.render_status(canvas, buf);
        self.render_edit_overlay(area, buf);
    }

    fn render_relations(&self, area: Rect, buf: &mut Buffer) {
        let Some(routes) = self.compute_oxdraw_routes() else {
            return;
        };
        for relation in &self.relations {
            let edge = relation_to_oxdraw_edge(relation);
            let edge_id = edge_identifier(&edge);
            let Some(route) = routes.get(&edge_id) else {
                continue;
            };
            self.draw_oxdraw_route(area, buf, route, relation.kind);
        }
    }

    fn render_notes(&self, area: Rect, buf: &mut Buffer) {
        let mut clusters = vec![Cluster::default(); area.width as usize * area.height as usize];

        for (index, note) in self.notes.iter().enumerate() {
            let projected = self.project_rect(note, area);
            if projected.w >= 8 && projected.h >= 5 {
                self.render_note_box(note, projected, area, buf, self.selected == Some(index));
            } else {
                self.add_cluster(note, area, &mut clusters, self.selected == Some(index));
            }
        }

        self.render_clusters(area, buf, &clusters);
    }

    fn render_note_box(
        &self,
        note: &Note,
        projected: ProjectedRect,
        area: Rect,
        buf: &mut Buffer,
        selected: bool,
    ) {
        let left = max(projected.x, 0);
        let top = max(projected.y, 0);
        let right = min(projected.x + projected.w, area.width as i32);
        let bottom = min(projected.y + projected.h, area.height as i32);
        if right - left < 2 || bottom - top < 2 {
            return;
        }

        let rect = Rect::new(
            area.x + left as u16,
            area.y + top as u16,
            (right - left) as u16,
            (bottom - top) as u16,
        );

        let border_style = Style::default().fg(self.note_color(note));
        if selected {
            for x in rect.x..rect.x + rect.width {
                let top = buf.cell_mut((x, rect.y)).expect("top border");
                if top.symbol() != " " {
                    top.set_style(top.style().add_modifier(Modifier::BOLD));
                }
                let bottom = buf
                    .cell_mut((x, rect.y + rect.height - 1))
                    .expect("bottom border");
                if bottom.symbol() != " " {
                    bottom.set_style(bottom.style().add_modifier(Modifier::BOLD));
                }
            }
        }

        Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(Line::from(vec![
                Span::styled(format!(" {} ", note.title), border_style),
                Span::styled(
                    format!("[{}]", note.key),
                    if selected {
                        Style::default()
                            .fg(Color::Black)
                            .bg(self.note_color(note))
                            .add_modifier(Modifier::BOLD)
                    } else {
                        border_style
                    },
                ),
                Span::raw(" "),
            ]))
            .render(rect, buf);

        if selected {
            for x in rect.x..rect.x + rect.width {
                let top = buf.cell_mut((x, rect.y)).expect("top border");
                if top.symbol() != " " {
                    top.set_style(top.style().add_modifier(Modifier::BOLD));
                }
                let bottom = buf
                    .cell_mut((x, rect.y + rect.height - 1))
                    .expect("bottom border");
                if bottom.symbol() != " " {
                    bottom.set_style(bottom.style().add_modifier(Modifier::BOLD));
                }
            }
            for y in rect.y..rect.y + rect.height {
                let left = buf.cell_mut((rect.x, y)).expect("left border");
                if left.symbol() != " " {
                    left.set_style(left.style().add_modifier(Modifier::BOLD));
                }
                let right = buf
                    .cell_mut((rect.x + rect.width - 1, y))
                    .expect("right border");
                if right.symbol() != " " {
                    right.set_style(right.style().add_modifier(Modifier::BOLD));
                }
            }
        }

        let inner = inset(rect, 1);
        if inner.width > 0 && inner.height > 0 {
            Paragraph::new(note.body.as_str())
                .style(Style::default().fg(Color::White))
                .render(inner, buf);
        }
    }

    fn add_cluster(&self, note: &Note, area: Rect, clusters: &mut [Cluster], selected: bool) {
        let (cx, cy) = self.project_point(note.x + note.w / 2.0, note.y + note.h / 2.0, area);
        if cx < 0 || cy < 0 || cx >= area.width as i32 || cy >= area.height as i32 {
            return;
        }

        let idx = cy as usize * area.width as usize + cx as usize;
        let cluster = &mut clusters[idx];
        cluster.count += 1;
        cluster.color = cluster.color.or(Some(self.note_color(note)));
        cluster.selected |= selected;
    }

    fn render_clusters(&self, area: Rect, buf: &mut Buffer, clusters: &[Cluster]) {
        for y in 0..area.height as usize {
            for x in 0..area.width as usize {
                let cluster = clusters[y * area.width as usize + x];
                if cluster.count == 0 {
                    continue;
                }

                let glyph = match cluster.count {
                    1 => '▪',
                    2..=3 => '◾',
                    _ => '◼',
                };
                let mut style = Style::default().fg(cluster.color.unwrap_or(Color::Gray));
                if cluster.selected {
                    style = style.bg(Color::DarkGray).add_modifier(Modifier::BOLD);
                } else if cluster.count >= 3 {
                    style = style.add_modifier(Modifier::BOLD);
                }
                buf.cell_mut((area.x + x as u16, area.y + y as u16))
                    .expect("cluster cell in bounds")
                    .set_char(glyph)
                    .set_style(style);
            }
        }
    }

    fn render_status(&self, area: Rect, buf: &mut Buffer) {
        let selection = self
            .selected
            .and_then(|idx| self.notes.get(idx))
            .map(|note| format!("sel [{}] {}", note.key, note.title))
            .unwrap_or_else(|| "pan".into());

        let mode = match &self.mode {
            Mode::Normal => self.status.as_str(),
            Mode::AwaitSelect => "awaiting node key",
            Mode::AwaitRelationPrefix(_) => "relation: press f then target key",
            Mode::AwaitRelationTarget(kind) => kind.label(),
            Mode::AwaitUnlinkPrefix => "unlink: press f then target key",
            Mode::AwaitUnlinkTarget => "unlink: target key",
            Mode::Edit(state) => match state.field {
                EditField::Title => "edit title  tab body  esc done",
                EditField::Body => "edit body  tab title  esc done",
            },
            Mode::Command => "",
        };
        let status = if matches!(self.mode, Mode::Command) {
            format!(":{}_", self.command_buffer)
        } else {
            format!(
                "{}  zoom {:.2}x  camera ({:.0}, {:.0})  {}{}",
                mode,
                self.zoom,
                self.camera_x,
                self.camera_y,
                selection,
                if self.dirty { "  *dirty" } else { "" }
            )
        };
        let mode_y = area.y + area.height.saturating_sub(1);
        fill_area(
            buf,
            Rect::new(area.x, mode_y, area.width, 1),
            ' ',
            Style::default().bg(Color::DarkGray).fg(Color::White),
        );
        buf.set_string(
            area.x + 1,
            mode_y,
            status,
            Style::default().bg(Color::DarkGray).fg(Color::White),
        );
    }

    fn render_edit_overlay(&self, area: Rect, buf: &mut Buffer) {
        let Mode::Edit(state) = &self.mode else {
            return;
        };
        let Some(selected) = self.selected else {
            return;
        };
        let Some(note) = self.notes.get(selected) else {
            return;
        };

        let width = min(area.width.saturating_sub(4), 70);
        let height = min(area.height.saturating_sub(4), 12);
        let rect = Rect::new(
            area.x + (area.width.saturating_sub(width)) / 2,
            area.y + (area.height.saturating_sub(height)) / 2,
            width,
            height,
        );
        Clear.render(rect, buf);
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" Edit [{}] ", note.key))
            .render(rect, buf);
        let inner = inset(rect, 1);
        if inner.width < 4 || inner.height < 4 {
            return;
        }

        let title_style = if matches!(state.field, EditField::Title) {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        let body_style = if matches!(state.field, EditField::Body) {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        buf.set_string(inner.x, inner.y, "Title:", Style::default().fg(Color::Gray));
        buf.set_string(inner.x + 7, inner.y, note.title.as_str(), title_style);
        if inner.height > 2 {
            buf.set_string(
                inner.x,
                inner.y + 2,
                "Body:",
                Style::default().fg(Color::Gray),
            );
            Paragraph::new(note.body.as_str()).style(body_style).render(
                Rect::new(inner.x, inner.y + 3, inner.width, inner.height - 3),
                buf,
            );
        }
    }

    fn start_edit(&mut self) {
        let Some(selected) = self.selected else {
            self.status = "select a node first".into();
            return;
        };
        let note = &self.notes[selected];
        self.mode = Mode::Edit(EditState {
            field: EditField::Title,
            title_cursor: note.title.len(),
            body_cursor: note.body.len(),
        });
        self.status = format!("editing {}", note.key);
    }

    fn apply_motion(&mut self, dx: f32, dy: f32, count: u16) {
        let repeat = max(1, count as i32) as f32;
        let step = self.movement_speed * repeat;
        if let Some(selected) = self.selected {
            let note_key = {
                let note = &mut self.notes[selected];
                note.x += dx * step;
                note.y += dy * step;
                note.key
            };
            self.keep_selected_note_visible();
            self.dirty = true;
            self.status = format!("moved [{}] by {},{}", note_key, dx * step, dy * step);
        } else {
            self.camera_x += (dx * 4.0 * step) / self.zoom;
            self.camera_y += (dy * 2.5 * step) / self.zoom;
        }
    }

    fn add_node(&mut self) {
        let Some(key) = self.next_key() else {
            self.status = "no available node keys".into();
            return;
        };
        let index = self.notes.len();
        self.notes.push(Note {
            key,
            title: String::new(),
            body: String::new(),
            x: self.camera_x - 8.0,
            y: self.camera_y - 3.0,
            w: 20.0,
            h: 8.0,
            color: NoteColor::cycle(index),
            file_path: Some(PathBuf::from(format!("nodes/{}.md", slugify_key(key)))),
        });
        self.selected = Some(index);
        self.mode = Mode::Edit(EditState {
            field: EditField::Title,
            title_cursor: 0,
            body_cursor: 0,
        });
        self.dirty = true;
        self.status = format!("added node [{}]", key);
    }

    fn select_by_key(&mut self, key: char) {
        if let Some(index) = self.notes.iter().position(|note| note.key == key) {
            self.selected = Some(index);
            self.mode = Mode::Normal;
            self.status = format!("selected [{}] {}", key, self.notes[index].title);
        } else {
            self.mode = Mode::Normal;
            self.status = format!("unknown node key [{}]", key);
        }
    }

    fn delete_selected_note(&mut self) {
        let Some(selected) = self.selected else {
            self.status = "select a node first".into();
            return;
        };

        let removed = self.notes.remove(selected);
        self.relations
            .retain(|relation| relation.from != removed.key && relation.to != removed.key);
        self.selected = None;
        self.mode = Mode::Normal;
        self.dirty = true;
        self.status = format!("deleted [{}]", removed.key);
    }

    fn begin_relation(&mut self, kind: RelationType) {
        if self.selected.is_none() {
            self.status = "select a source node first".into();
            return;
        }
        self.mode = Mode::AwaitRelationPrefix(kind);
        self.status = format!("{} relation: press f then target key", kind.label());
    }

    fn begin_unlink(&mut self) {
        if self.selected.is_none() {
            self.status = "select a source node first".into();
            return;
        }
        self.mode = Mode::AwaitUnlinkPrefix;
        self.status = "unlink: press f then target key".into();
    }

    fn create_relation_to(&mut self, key: char, kind: RelationType) {
        let Some(selected) = self.selected else {
            self.mode = Mode::Normal;
            return;
        };
        let from_key = self.notes[selected].key;
        if self.notes.iter().all(|note| note.key != key) {
            self.status = format!("unknown target [{}]", key);
            self.mode = Mode::Normal;
            return;
        }
        self.relations.push(Relation {
            from: from_key,
            to: key,
            kind,
        });
        self.dirty = true;
        self.mode = Mode::Normal;
        self.status = format!("linked [{}] -> [{}] ({})", from_key, key, kind.label());
    }

    fn remove_relation_to(&mut self, key: char) {
        let Some(selected) = self.selected else {
            self.mode = Mode::Normal;
            return;
        };
        let from_key = self.notes[selected].key;
        let before = self.relations.len();
        self.relations
            .retain(|relation| !(relation.from == from_key && relation.to == key));
        let removed = before.saturating_sub(self.relations.len());
        self.mode = Mode::Normal;
        if removed == 0 {
            self.status = format!("no relation [{}] -> [{}]", from_key, key);
            return;
        }
        self.dirty = true;
        self.status = format!("unlinked [{}] -> [{}]", from_key, key);
    }

    fn cancel_pending(&mut self) {
        match self.mode {
            Mode::Normal => {
                self.selected = None;
                self.status = "selection cleared".into();
            }
            _ => {
                self.mode = Mode::Normal;
                self.count_buffer.clear();
                self.command_buffer.clear();
                self.status = "cancelled".into();
            }
        }
    }

    fn zoom_by(&mut self, factor: f32) {
        self.zoom = (self.zoom * factor).clamp(0.08, 4.0);
    }

    fn fit_all(&mut self) {
        if self.notes.is_empty() || self.last_canvas.width < 2 || self.last_canvas.height < 2 {
            return;
        }
        let mut min_x = f32::INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        for note in &self.notes {
            min_x = min_x.min(note.x);
            min_y = min_y.min(note.y);
            max_x = max_x.max(note.x + note.w);
            max_y = max_y.max(note.y + note.h);
        }
        self.camera_x = (min_x + max_x) / 2.0;
        self.camera_y = (min_y + max_y) / 2.0;
        let width = (max_x - min_x).max(1.0) + 8.0;
        let height = (max_y - min_y).max(1.0) + 6.0;
        let zoom_x = self.last_canvas.width as f32 / width;
        let zoom_y = self.last_canvas.height as f32 / height;
        self.zoom = zoom_x.min(zoom_y).clamp(0.08, 4.0);
        self.status = "fit all nodes".into();
    }

    fn take_count(&mut self) -> u16 {
        let value = self.count_buffer.parse::<u16>().unwrap_or(1);
        self.count_buffer.clear();
        value
    }

    fn open_selected_in_editor(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    ) -> Result<()> {
        let Some(selected) = self.selected else {
            self.status = "select a node first".into();
            return Ok(());
        };

        let path = temp_note_markdown_path(self.notes[selected].key);
        self.sync_note_to_markdown(selected, &path)?;
        self.run_external_command(
            terminal,
            &format!("{} {}", self.editor_command, shell_quote(&path)),
        )?;
        self.sync_note_from_markdown(selected, &path)?;
        self.status = format!("opened {}", path.display());
        Ok(())
    }

    fn sync_note_to_markdown(&self, index: usize, path: &Path) -> Result<()> {
        let note = &self.notes[index];
        let content = if note.body.trim().is_empty() {
            format!("# {}\n", note.title)
        } else {
            format!("# {}\n\n{}", note.title, note.body)
        };
        fs::write(path, content).with_context(|| format!("failed to write {}", path.display()))
    }

    fn sync_note_from_markdown(&mut self, index: usize, path: &Path) -> Result<()> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let mut lines = content.lines();
        let first = lines.next().unwrap_or("").trim();
        let title = first.strip_prefix("# ").unwrap_or(first).trim().to_string();
        let body = lines
            .collect::<Vec<_>>()
            .join("\n")
            .trim_start_matches('\n')
            .to_string();
        let note = &mut self.notes[index];
        if !title.is_empty() {
            note.title = title;
        }
        note.body = body;
        self.dirty = true;
        Ok(())
    }

    fn run_external_command(
        &self,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
        command: &str,
    ) -> Result<()> {
        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        let status = Command::new("/bin/zsh")
            .arg("-lc")
            .arg(command)
            .status()
            .with_context(|| format!("failed to run external command: {command}"))?;
        execute!(
            terminal.backend_mut(),
            EnterAlternateScreen,
            EnableMouseCapture
        )?;
        enable_raw_mode()?;
        terminal.clear()?;
        if !status.success() {
            return Err(anyhow!("external command failed: {command}"));
        }
        Ok(())
    }

    fn save_loaded(&mut self) -> Result<()> {
        let path = self.target_save_path();
        self.save_to_path(&path)?;
        self.loaded_path = Some(path.clone());
        self.status = format!("saved {}", path.display());
        Ok(())
    }

    fn save_to_path(&mut self, path: &Path) -> Result<()> {
        let target = if path.extension().and_then(|ext| ext.to_str()) == Some("mmd") {
            path.to_path_buf()
        } else {
            path.with_extension("mmd")
        };
        fs::write(&target, self.to_mmd())
            .with_context(|| format!("failed to write {}", target.display()))?;
        self.dirty = false;
        Ok(())
    }

    fn export_mermaid(&mut self) -> Result<PathBuf> {
        let path = if let Some(loaded) = &self.loaded_path {
            loaded.with_extension("mmd")
        } else {
            PathBuf::from("graph.mmd")
        };
        fs::write(&path, self.to_mmd())
            .with_context(|| format!("failed to write {}", path.display()))?;
        Ok(path)
    }

    fn to_mmd(&self) -> String {
        let out = self.to_mermaid_definition();
        let layout = SavedLayoutOverrides {
            nodes: self
                .notes
                .iter()
                .map(|note| {
                    (
                        note.key.to_string(),
                        SavedPoint {
                            x: note.x,
                            y: note.y,
                        },
                    )
                })
                .collect(),
        };
        merge_mmd_and_layout(&out, &layout).unwrap_or(out)
    }

    fn to_mermaid_definition(&self) -> String {
        let mut out = String::from("graph LR\n");
        for note in &self.notes {
            out.push_str(&format!(
                "    {}[{}]\n",
                note.key,
                escape_mermaid(&note_markdown_label(note))
            ));
        }
        for relation in &self.relations {
            out.push_str(&format!(
                "    {} {} {}\n",
                relation.from,
                relation.kind.mermaid_operator(),
                relation.to
            ));
        }
        out
    }

    fn quit_requested(&mut self, force: bool) {
        if self.dirty && !force {
            self.status = "unsaved changes; use :w to save or :q! to force".into();
            return;
        }
        self.should_quit = true;
    }

    fn load_config(&mut self) -> Result<()> {
        let path = config_path()?;
        if !path.exists() {
            return Ok(());
        }
        let data = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let config: AppConfig =
            serde_json::from_str(&data).context("failed to parse app config")?;
        if let Some(editor) = config.editor_command {
            self.editor_command = editor;
        }
        if let Some(monocolor) = config.monocolor {
            self.monocolor = monocolor;
        }
        if let Some(movement_speed) = config.movement_speed {
            self.movement_speed = movement_speed.max(0.1);
        }
        Ok(())
    }

    fn save_config(&self) -> Result<()> {
        let path = config_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let config = AppConfig {
            editor_command: Some(self.editor_command.clone()),
            monocolor: Some(self.monocolor),
            movement_speed: Some(self.movement_speed),
        };
        fs::write(&path, serde_json::to_string_pretty(&config)?)
            .with_context(|| format!("failed to write {}", path.display()))?;
        Ok(())
    }

    fn note_color(&self, note: &Note) -> Color {
        if self.monocolor {
            Color::Indexed(231)
        } else {
            note.color.ratatui()
        }
    }

    fn next_key(&self) -> Option<char> {
        let used: Vec<char> = self.notes.iter().map(|note| note.key).collect();
        ('a'..='z')
            .chain('0'..='9')
            .find(|candidate| !used.contains(candidate))
    }

    fn target_save_path(&self) -> PathBuf {
        match &self.loaded_path {
            Some(path) if path.extension().and_then(|ext| ext.to_str()) == Some("mmd") => {
                path.clone()
            }
            Some(path) => path.with_extension("mmd"),
            None => PathBuf::from("graph.mmd"),
        }
    }

    fn keep_selected_note_visible(&mut self) {
        let Some(selected) = self.selected else {
            return;
        };
        if self.last_canvas.width < 4 || self.last_canvas.height < 4 {
            return;
        }
        let Some(note) = self.notes.get(selected) else {
            return;
        };

        let projected = self.project_rect(note, self.last_canvas);
        let margin_x = 2;
        let margin_y = 1;
        let left = projected.x;
        let right = projected.x + projected.w;
        let top = projected.y;
        let bottom = projected.y + projected.h;
        let min_x = margin_x;
        let max_x = self.last_canvas.width as i32 - margin_x;
        let min_y = margin_y;
        let max_y = self.last_canvas.height as i32 - margin_y;

        if left < min_x {
            self.camera_x -= (min_x - left) as f32 / self.zoom;
        } else if right > max_x {
            self.camera_x += (right - max_x) as f32 / self.zoom;
        }

        if top < min_y {
            self.camera_y -= (min_y - top) as f32 / self.zoom;
        } else if bottom > max_y {
            self.camera_y += (bottom - max_y) as f32 / self.zoom;
        }
    }

    fn project_rect(&self, note: &Note, area: Rect) -> ProjectedRect {
        let x = ((note.x - self.camera_x) * self.zoom + area.width as f32 / 2.0).round() as i32;
        let y = ((note.y - self.camera_y) * self.zoom + area.height as f32 / 2.0).round() as i32;
        let w = max(1, (note.w * self.zoom).round() as i32);
        let h = max(1, (note.h * self.zoom).round() as i32);
        ProjectedRect { x, y, w, h }
    }

    fn project_point(&self, x: f32, y: f32, area: Rect) -> (i32, i32) {
        let sx = ((x - self.camera_x) * self.zoom + area.width as f32 / 2.0).round() as i32;
        let sy = ((y - self.camera_y) * self.zoom + area.height as f32 / 2.0).round() as i32;
        (sx, sy)
    }

    fn draw_tip(&self, area: Rect, buf: &mut Buffer, x: i32, y: i32, glyph: char) {
        if glyph == ' ' || x < 0 || y < 0 || x >= area.width as i32 || y >= area.height as i32 {
            return;
        }
        buf.cell_mut((area.x + x as u16, area.y + y as u16))
            .expect("tip cell in bounds")
            .set_char(glyph)
            .set_style(Style::default().fg(Color::Gray));
    }

    fn draw_oxdraw_route(
        &self,
        area: Rect,
        buf: &mut Buffer,
        route: &[OxPoint],
        relation_kind: RelationType,
    ) {
        if route.len() < 2 {
            return;
        }
        let screen_points: Vec<(i32, i32)> = route
            .iter()
            .filter_map(ox_point_components)
            .map(|point| self.project_point(point.x, point.y, area))
            .collect();
        if screen_points.len() < 2 {
            return;
        }
        draw_polyline(
            buf,
            area,
            &screen_points,
            relation_kind.stroke(),
            Style::default().fg(Color::DarkGray),
        );
        let start = screen_points[0];
        let start_next = screen_points[1];
        let end = *screen_points.last().expect("route has at least 2 points");
        let end_prev = screen_points[screen_points.len() - 2];
        self.draw_tip(
            area,
            buf,
            start.0,
            start.1,
            relation_kind.tip(true, start_next.0 - start.0, start_next.1 - start.1),
        );
        self.draw_tip(
            area,
            buf,
            end.0,
            end.1,
            relation_kind.tip(false, end.0 - end_prev.0, end.1 - end_prev.1),
        );
    }

    fn compute_oxdraw_routes(&self) -> Option<HashMap<String, Vec<OxPoint>>> {
        let definition = self.to_mermaid_definition();
        let mut diagram = OxDiagram::parse(&definition).ok()?;
        for note in &self.notes {
            let id = note.key.to_string();
            if let Some(node) = diagram.nodes.get_mut(&id) {
                node.width = note.w;
                node.height = note.h;
            }
        }
        let mut node_overrides = HashMap::new();
        for note in &self.notes {
            node_overrides.insert(
                note.key.to_string(),
                make_ox_point(note.x + note.w / 2.0, note.y + note.h / 2.0)?,
            );
        }
        let overrides = OxLayoutOverrides {
            nodes: node_overrides,
            ..OxLayoutOverrides::default()
        };
        Some(diagram.layout(Some(&overrides)).ok()?.final_routes)
    }
}

#[derive(Clone, Copy)]
struct ProjectedRect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

#[derive(Clone, Copy, Default)]
struct Cluster {
    count: u16,
    color: Option<Color>,
    selected: bool,
}

fn parse_mermaid_graph(source: &str) -> Result<GraphFile> {
    let mut notes = Vec::new();
    let mut relations = Vec::new();
    let mut index = 0usize;

    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("graph ") {
            continue;
        }

        if let Some((left, title)) = parse_mermaid_node_line(trimmed) {
            let key = left
                .trim()
                .chars()
                .next()
                .ok_or_else(|| anyhow!("missing node key"))?;
            notes.push(Note {
                key,
                title: title.into(),
                body: String::new(),
                x: (index as f32 % 4.0) * 26.0,
                y: (index as f32 / 4.0).floor() * 12.0,
                w: 20.0,
                h: 8.0,
                color: NoteColor::cycle(index),
                file_path: None,
            });
            index += 1;
            continue;
        }

        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() == 3 {
            let from = parts[0]
                .chars()
                .next()
                .ok_or_else(|| anyhow!("missing from key"))?;
            let to = parts[2]
                .chars()
                .next()
                .ok_or_else(|| anyhow!("missing to key"))?;
            let kind = match parts[1] {
                "<-->" => RelationType::Bidirectional,
                "--o" => RelationType::Compositional,
                "-.-" => RelationType::Cluster,
                _ => RelationType::Directional,
            };
            relations.push(Relation { from, to, kind });
        }
    }

    Ok(GraphFile {
        notes,
        relations,
        editor_command: None,
    })
}

fn parse_mermaid_node_line(line: &str) -> Option<(&str, String)> {
    let (left, right) = line.split_once('[')?;
    let label = right.strip_suffix(']')?;
    let label = label
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .unwrap_or(label);
    Some((left, label.to_string()))
}

fn parse_saved_mmd(source: &str) -> Result<GraphFile> {
    let (definition, layout) = split_saved_layout(source)?;
    let mut graph = parse_mermaid_graph(&definition)?;
    for note in &mut graph.notes {
        if let Some(point) = layout.nodes.get(&note.key.to_string()) {
            note.x = point.x;
            note.y = point.y;
        }
        let (title, body) = split_saved_label(&note.title);
        note.title = title;
        note.body = body;
    }
    Ok(graph)
}

fn relation_to_oxdraw_edge(relation: &Relation) -> OxEdge {
    let (kind, arrow) = match relation.kind {
        RelationType::Directional => (OxEdgeKind::Solid, OxArrowDirection::Forward),
        RelationType::Bidirectional => (OxEdgeKind::Solid, OxArrowDirection::Both),
        RelationType::Compositional => (OxEdgeKind::Thick, OxArrowDirection::Forward),
        RelationType::Cluster => (OxEdgeKind::Dashed, OxArrowDirection::None),
    };
    OxEdge {
        from: relation.from.to_string(),
        to: relation.to.to_string(),
        label: None,
        kind,
        arrow,
    }
}

fn draw_line(
    buf: &mut Buffer,
    area: Rect,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    glyph: char,
    style: Style,
) {
    let glyph = if x0 == x1 {
        '|'
    } else if y0 == y1 {
        glyph
    } else {
        glyph
    };
    let mut x = x0;
    let mut y = y0;
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;

    loop {
        if x >= 0 && y >= 0 && x < area.width as i32 && y < area.height as i32 {
            let cell = buf
                .cell_mut((area.x + x as u16, area.y + y as u16))
                .expect("line cell in bounds");
            if matches!(cell.symbol(), " " | "·") {
                cell.set_char(glyph).set_style(style);
            }
        }
        if x == x1 && y == y1 {
            break;
        }
        let err2 = 2 * err;
        if err2 >= dy {
            err += dy;
            x += sx;
        }
        if err2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

fn split_saved_layout(source: &str) -> Result<(String, SavedLayoutOverrides)> {
    const START: &str = "%% OXDRAW LAYOUT START";
    const END: &str = "%% OXDRAW LAYOUT END";

    let mut definition_lines = Vec::new();
    let mut layout_lines = Vec::new();
    let mut in_block = false;

    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.eq_ignore_ascii_case(START) {
            in_block = true;
            continue;
        }
        if trimmed.eq_ignore_ascii_case(END) {
            in_block = false;
            continue;
        }
        if in_block {
            let mut segment = line.trim_start();
            if let Some(rest) = segment.strip_prefix("%%") {
                segment = rest.trim_start();
            }
            if !segment.trim().is_empty() {
                layout_lines.push(segment.to_string());
            }
        } else {
            definition_lines.push(line);
        }
    }

    let mut definition = definition_lines.join("\n");
    if source.ends_with('\n') {
        definition.push('\n');
    }
    let layout = if layout_lines.is_empty() {
        SavedLayoutOverrides::default()
    } else {
        serde_json::from_str(&layout_lines.join("\n"))
            .context("failed to parse embedded oxdraw layout block")?
    };
    Ok((definition, layout))
}

fn merge_mmd_and_layout(definition: &str, layout: &SavedLayoutOverrides) -> Result<String> {
    let trimmed = definition.trim_end_matches('\n');
    let mut output = trimmed.to_string();
    output.push('\n');
    if layout.nodes.is_empty() {
        return Ok(output);
    }
    let json = serde_json::to_string_pretty(layout)?;
    output.push('\n');
    output.push_str("%% OXDRAW LAYOUT START\n");
    for line in json.lines() {
        output.push_str("%% ");
        output.push_str(line);
        output.push('\n');
    }
    output.push_str("%% OXDRAW LAYOUT END\n");
    Ok(output)
}

fn draw_polyline(buf: &mut Buffer, area: Rect, points: &[(i32, i32)], glyph: char, style: Style) {
    for pair in points.windows(2) {
        let [(x0, y0), (x1, y1)] = [pair[0], pair[1]];
        let segment_glyph = if x0 == x1 { '|' } else { glyph };
        draw_line(buf, area, x0, y0, x1, y1, segment_glyph, style);
    }
}

fn inset(area: Rect, margin: u16) -> Rect {
    let horizontal = margin.saturating_mul(2);
    let vertical = margin.saturating_mul(2);
    if area.width <= horizontal || area.height <= vertical {
        return Rect::new(area.x, area.y, 0, 0);
    }
    Rect::new(
        area.x + margin,
        area.y + margin,
        area.width - horizontal,
        area.height - vertical,
    )
}

fn fill_area(buf: &mut Buffer, area: Rect, ch: char, style: Style) {
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            buf.cell_mut((x, y))
                .expect("fill cell in bounds")
                .set_char(ch)
                .set_style(style);
        }
    }
}

fn content_area(area: Rect) -> Rect {
    if area.height <= 1 {
        area
    } else {
        Rect::new(area.x, area.y, area.width, area.height - 1)
    }
}

fn shell_quote(path: &Path) -> String {
    let raw = path.display().to_string();
    format!("'{}'", raw.replace('\'', "'\\''"))
}

fn config_path() -> Result<PathBuf> {
    if let Ok(dir) = env::var("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(dir).join("oxmap").join("config.json"));
    }
    let home = env::var("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home)
        .join(".config")
        .join("oxmap")
        .join("config.json"))
}

fn move_cursor(state: &mut EditState, delta: isize, len: usize) {
    let cursor = match state.field {
        EditField::Title => &mut state.title_cursor,
        EditField::Body => &mut state.body_cursor,
    };
    let next = (*cursor as isize + delta).clamp(0, len as isize);
    *cursor = next as usize;
}

fn slugify_key(key: char) -> String {
    key.to_string()
}

fn escape_mermaid(text: &str) -> String {
    text.replace('"', "\\\"")
}

fn note_markdown_label(note: &Note) -> String {
    if note.body.trim().is_empty() {
        note.title.replace('\n', "<br/>")
    } else {
        format!(
            "{}<br/>{}",
            note.title.replace('\n', "<br/>"),
            note.body.replace('\n', "<br/>")
        )
    }
}

fn split_saved_label(label: &str) -> (String, String) {
    let normalized = label.replace("<br/>", "\n").replace("<br>", "\n");
    let mut lines = normalized.lines();
    let title = lines.next().unwrap_or("").trim().to_string();
    let body = lines.collect::<Vec<_>>().join("\n").trim().to_string();
    (title, body)
}

fn temp_note_markdown_path(key: char) -> PathBuf {
    env::temp_dir().join(format!("oxmap-node-{key}.md"))
}

fn ox_point_components(point: &OxPoint) -> Option<OxPointView> {
    serde_json::to_value(point)
        .ok()
        .and_then(|value| serde_json::from_value(value).ok())
}

fn make_ox_point(x: f32, y: f32) -> Option<OxPoint> {
    serde_json::from_value(serde_json::json!({ "x": x, "y": y })).ok()
}

fn arrow_glyph(dx: i32, dy: i32) -> char {
    if dx.abs() >= dy.abs() {
        if dx >= 0 { '▶' } else { '◀' }
    } else {
        if dy >= 0 { '▼' } else { '▲' }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsaved_quit_stays_in_app() {
        let mut app = App::demo();
        app.dirty = true;

        app.run_command("q").expect("q command should not error");

        assert!(!app.should_quit);
        assert!(app.status.contains("unsaved changes"));
    }

    #[test]
    fn wq_writes_graph_mmd_for_fresh_session() {
        let original_dir = env::current_dir().expect("current dir");
        let temp_dir = env::temp_dir().join(format!(
            "oxmap-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        env::set_current_dir(&temp_dir).expect("enter temp dir");

        let graph_path = temp_dir.join("graph.mmd");
        let mut app = App::demo();
        app.dirty = true;

        let result = app.run_command("wq");

        env::set_current_dir(&original_dir).expect("restore cwd");

        assert!(result.is_ok());
        assert!(app.should_quit);
        assert!(graph_path.exists());

        let _ = fs::remove_file(&graph_path);
        let _ = fs::remove_dir(&temp_dir);
    }

    #[test]
    fn mermaid_export_uses_oxdraw_compatible_syntax() {
        let mut app = App::demo();
        app.notes = vec![Note {
            key: 'a',
            title: "hello world".into(),
            body: String::new(),
            x: 0.0,
            y: 0.0,
            w: 20.0,
            h: 8.0,
            color: NoteColor::White,
            file_path: None,
        }];
        app.relations = vec![Relation {
            from: 'a',
            to: 'a',
            kind: RelationType::Cluster,
        }];

        let output = app.to_mermaid_definition();

        assert!(output.contains("a[hello world]"));
        assert!(output.contains("a --> a"));
        assert!(!output.contains("[\""));
        assert!(!output.contains("-.-"));
    }

    #[test]
    fn saved_mmd_round_trips_note_positions() {
        let mut app = App::demo();
        app.notes = vec![Note {
            key: 'a',
            title: "hello world".into(),
            body: "body text".into(),
            x: 123.5,
            y: -48.25,
            w: 20.0,
            h: 8.0,
            color: NoteColor::White,
            file_path: None,
        }];
        app.relations.clear();

        let saved = app.to_mmd();
        let loaded = parse_saved_mmd(&saved).expect("saved mmd should parse");

        assert_eq!(loaded.notes.len(), 1);
        assert_eq!(loaded.notes[0].title, "hello world");
        assert_eq!(loaded.notes[0].body, "body text");
        assert!((loaded.notes[0].x - 123.5).abs() < f32::EPSILON);
        assert!((loaded.notes[0].y - (-48.25)).abs() < f32::EPSILON);
    }

    #[test]
    fn moving_selected_note_keeps_it_on_screen() {
        let mut app = App::demo();
        app.notes = vec![Note {
            key: 'a',
            title: "hello".into(),
            body: String::new(),
            x: 0.0,
            y: 0.0,
            w: 20.0,
            h: 8.0,
            color: NoteColor::White,
            file_path: None,
        }];
        app.selected = Some(0);
        app.last_canvas = Rect::new(0, 0, 40, 20);
        app.zoom = 1.0;
        app.camera_x = 0.0;
        app.camera_y = 0.0;
        app.movement_speed = 2.0;

        app.apply_motion(1.0, 0.0, 20);

        let projected = app.project_rect(&app.notes[0], app.last_canvas);
        assert!(projected.x + projected.w <= app.last_canvas.width as i32 - 2);
    }

    #[test]
    fn deleting_selected_note_removes_attached_relations() {
        let mut app = App::demo();
        app.notes = vec![
            Note {
                key: 'a',
                title: "a".into(),
                body: String::new(),
                x: 0.0,
                y: 0.0,
                w: 20.0,
                h: 8.0,
                color: NoteColor::White,
                file_path: None,
            },
            Note {
                key: 'b',
                title: "b".into(),
                body: String::new(),
                x: 10.0,
                y: 10.0,
                w: 20.0,
                h: 8.0,
                color: NoteColor::White,
                file_path: None,
            },
        ];
        app.relations = vec![Relation {
            from: 'a',
            to: 'b',
            kind: RelationType::Directional,
        }];
        app.selected = Some(0);

        app.delete_selected_note();

        assert_eq!(app.notes.len(), 1);
        assert_eq!(app.notes[0].key, 'b');
        assert!(app.relations.is_empty());
        assert!(app.selected.is_none());
    }

    #[test]
    fn removing_relation_to_target_deletes_only_matching_edge() {
        let mut app = App::demo();
        app.notes = vec![
            Note {
                key: 'a',
                title: "a".into(),
                body: String::new(),
                x: 0.0,
                y: 0.0,
                w: 20.0,
                h: 8.0,
                color: NoteColor::White,
                file_path: None,
            },
            Note {
                key: 'b',
                title: "b".into(),
                body: String::new(),
                x: 10.0,
                y: 10.0,
                w: 20.0,
                h: 8.0,
                color: NoteColor::White,
                file_path: None,
            },
            Note {
                key: 'c',
                title: "c".into(),
                body: String::new(),
                x: 20.0,
                y: 20.0,
                w: 20.0,
                h: 8.0,
                color: NoteColor::White,
                file_path: None,
            },
        ];
        app.relations = vec![
            Relation {
                from: 'a',
                to: 'b',
                kind: RelationType::Directional,
            },
            Relation {
                from: 'a',
                to: 'c',
                kind: RelationType::Directional,
            },
        ];
        app.selected = Some(0);

        app.remove_relation_to('b');

        assert_eq!(app.relations.len(), 1);
        assert_eq!(app.relations[0].to, 'c');
        assert!(app.status.contains("unlinked"));
    }

    #[test]
    fn config_accepts_editor_and_movement_step_keys() {
        let config: AppConfig = serde_json::from_str(
            r#"{
                "editor": "~/nvim-macos-arm64/bin/nvim",
                "movement_step": 6
            }"#,
        )
        .expect("config should parse");

        assert_eq!(
            config.editor_command.as_deref(),
            Some("~/nvim-macos-arm64/bin/nvim")
        );
        assert_eq!(config.movement_speed, Some(6.0));
    }
}
