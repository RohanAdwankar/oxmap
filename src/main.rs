use std::cmp::{max, min};
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
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::border;
use ratatui::text::Line;
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
            app.last_canvas = inset(frame.area(), 1);
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
            Self::Cyan => Color::Cyan,
            Self::Yellow => Color::Yellow,
            Self::Green => Color::Green,
            Self::Magenta => Color::Magenta,
            Self::Blue => Color::Blue,
            Self::Red => Color::Red,
            Self::White => Color::White,
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

    fn tip(self, from_start: bool) -> char {
        match self {
            Self::Directional => {
                if from_start {
                    ' '
                } else {
                    '▶'
                }
            }
            Self::Bidirectional => {
                if from_start {
                    '◀'
                } else {
                    '▶'
                }
            }
            Self::Compositional => {
                if from_start {
                    '◆'
                } else {
                    '▶'
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
            Self::Cluster => "-.-",
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

enum Mode {
    Normal,
    AwaitSelect,
    AwaitRelationPrefix(RelationType),
    AwaitRelationTarget(RelationType),
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
    loaded_path: Option<PathBuf>,
    dirty: bool,
    should_quit: bool,
    last_canvas: Rect,
}

impl App {
    fn load(args: Args) -> Result<Self> {
        let mut app = Self::demo();
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
            notes: vec![
                Note {
                    key: 'a',
                    title: "Inbox".into(),
                    body: "quick capture\nmeeting notes".into(),
                    x: -30.0,
                    y: -8.0,
                    w: 20.0,
                    h: 8.0,
                    color: NoteColor::Cyan,
                    file_path: Some(PathBuf::from("nodes/inbox.md")),
                },
                Note {
                    key: 'd',
                    title: "Design".into(),
                    body: "progressive detail\nzoomed canvas".into(),
                    x: -2.0,
                    y: -12.0,
                    w: 24.0,
                    h: 9.0,
                    color: NoteColor::Yellow,
                    file_path: Some(PathBuf::from("nodes/design.md")),
                },
                Note {
                    key: 'p',
                    title: "API".into(),
                    body: "cluster nearby cards\nbefore text vanishes".into(),
                    x: 29.0,
                    y: -4.0,
                    w: 25.0,
                    h: 8.0,
                    color: NoteColor::Green,
                    file_path: Some(PathBuf::from("nodes/api.md")),
                },
                Note {
                    key: 'r',
                    title: "Roadmap".into(),
                    body: "terminal map\nobsidian-style overview".into(),
                    x: -18.0,
                    y: 10.0,
                    w: 22.0,
                    h: 8.0,
                    color: NoteColor::Magenta,
                    file_path: Some(PathBuf::from("nodes/roadmap.md")),
                },
                Note {
                    key: 'i',
                    title: "Ideas".into(),
                    body: "semantic zoom\n1-char glyphs when far away".into(),
                    x: 15.0,
                    y: 13.0,
                    w: 26.0,
                    h: 9.0,
                    color: NoteColor::Blue,
                    file_path: Some(PathBuf::from("nodes/ideas.md")),
                },
                Note {
                    key: 'l',
                    title: "Links".into(),
                    body: "future work:\nedge routing".into(),
                    x: 52.0,
                    y: 16.0,
                    w: 18.0,
                    h: 7.0,
                    color: NoteColor::Red,
                    file_path: Some(PathBuf::from("nodes/links.md")),
                },
            ],
            relations: vec![
                Relation {
                    from: 'd',
                    to: 'i',
                    kind: RelationType::Directional,
                },
                Relation {
                    from: 'r',
                    to: 'l',
                    kind: RelationType::Cluster,
                },
            ],
            selected: None,
            mode: Mode::Normal,
            camera_x: 10.0,
            camera_y: 5.0,
            zoom: 1.4,
            count_buffer: String::new(),
            command_buffer: String::new(),
            status: "a add  f<key> select  m f <key> relate  : commands".into(),
            editor_command: env::var("EDITOR").unwrap_or_else(|_| "vi".into()),
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
                let graph = parse_mermaid_graph(&data)?;
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
            KeyCode::Char('q') => self.quit_requested(false)?,
            KeyCode::Char(':') => {
                self.mode = Mode::Command;
                self.command_buffer.clear();
                self.status = "command mode".into();
            }
            KeyCode::Char('a') => self.add_node(),
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
            "q" => self.quit_requested(false)?,
            "q!" => self.should_quit = true,
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
        Block::default()
            .borders(Borders::ALL)
            .border_set(border::THICK)
            .title(" Note Graph ")
            .title_bottom(Line::from(
                " a add  f select  i edit  o open  m link  s/d zoom  : command ",
            ))
            .render(area, buf);

        let canvas = inset(area, 1);
        if canvas.width < 4 || canvas.height < 4 {
            return;
        }

        fill_area(buf, canvas, ' ', Style::default().bg(Color::Black));
        self.render_grid(canvas, buf);
        self.render_relations(canvas, buf);
        self.render_notes(canvas, buf);
        self.render_status(canvas, buf);
        self.render_command_bar(area, buf);
        self.render_edit_overlay(area, buf);
    }

    fn render_grid(&self, area: Rect, buf: &mut Buffer) {
        let world_step = 10.0;
        if self.zoom * world_step < 4.0 {
            return;
        }

        let left_world = self.camera_x - (area.width as f32 / self.zoom) / 2.0;
        let top_world = self.camera_y - (area.height as f32 / self.zoom) / 2.0;
        let first_x = (left_world / world_step).floor() as i32 - 1;
        let first_y = (top_world / world_step).floor() as i32 - 1;

        for gx in first_x..first_x + 64 {
            let world_x = gx as f32 * world_step;
            let sx = ((world_x - self.camera_x) * self.zoom + area.width as f32 / 2.0).round();
            let x = area.x as i32 + sx as i32;
            if x < area.x as i32 || x >= (area.x + area.width) as i32 {
                continue;
            }
            for y in area.y..area.y + area.height {
                let cell = buf.cell_mut((x as u16, y)).expect("grid cell in bounds");
                if cell.symbol() == " " {
                    cell.set_char('·')
                        .set_style(Style::default().fg(Color::DarkGray));
                }
            }
        }

        for gy in first_y..first_y + 64 {
            let world_y = gy as f32 * world_step;
            let sy = ((world_y - self.camera_y) * self.zoom + area.height as f32 / 2.0).round();
            let y = area.y as i32 + sy as i32;
            if y < area.y as i32 || y >= (area.y + area.height) as i32 {
                continue;
            }
            for x in area.x..area.x + area.width {
                let cell = buf.cell_mut((x, y as u16)).expect("grid cell in bounds");
                if cell.symbol() == " " {
                    cell.set_char('·')
                        .set_style(Style::default().fg(Color::DarkGray));
                }
            }
        }
    }

    fn render_relations(&self, area: Rect, buf: &mut Buffer) {
        for relation in &self.relations {
            let Some(from) = self.node_by_key(relation.from) else {
                continue;
            };
            let Some(to) = self.node_by_key(relation.to) else {
                continue;
            };

            let (x0, y0) = self.project_point(from.x + from.w / 2.0, from.y + from.h / 2.0, area);
            let (x1, y1) = self.project_point(to.x + to.w / 2.0, to.y + to.h / 2.0, area);
            draw_line(
                buf,
                area,
                x0,
                y0,
                x1,
                y1,
                relation.kind.stroke(),
                Style::default().fg(Color::DarkGray),
            );
            self.draw_tip(area, buf, x0, y0, relation.kind.tip(true));
            self.draw_tip(area, buf, x1, y1, relation.kind.tip(false));
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

        let mut border_style = Style::default().fg(note.color.ratatui());
        if selected {
            border_style = border_style
                .add_modifier(Modifier::BOLD)
                .bg(Color::DarkGray);
        }

        Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(Line::styled(
                format!(" {} [{}] ", note.title, note.key),
                border_style,
            ))
            .render(rect, buf);

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
        cluster.color = cluster.color.or(Some(note.color.ratatui()));
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
            .map(|note| format!(" selected [{}] {}", note.key, note.title))
            .unwrap_or_else(|| " pan mode".into());
        let status = format!(
            "zoom {:.2}x  camera ({:.0}, {:.0}){}{}",
            self.zoom,
            self.camera_x,
            self.camera_y,
            selection,
            if self.dirty { "  *dirty" } else { "" }
        );
        let width = min(status.chars().count() as u16 + 2, area.width);
        let status_area = Rect::new(area.x, area.y, width, 1);
        fill_area(
            buf,
            status_area,
            ' ',
            Style::default().bg(Color::DarkGray).fg(Color::White),
        );
        buf.set_string(
            area.x + 1,
            area.y,
            status,
            Style::default().bg(Color::DarkGray).fg(Color::White),
        );

        let mode = match &self.mode {
            Mode::Normal => self.status.as_str(),
            Mode::AwaitSelect => "awaiting node key",
            Mode::AwaitRelationPrefix(kind) => match kind {
                RelationType::Directional => "relation: press f then target key",
                _ => "typed relation: press f then target key",
            },
            Mode::AwaitRelationTarget(kind) => match kind {
                RelationType::Directional => "relation target",
                RelationType::Bidirectional => "bidirectional target",
                RelationType::Compositional => "compositional target",
                RelationType::Cluster => "cluster target",
            },
            Mode::Edit(state) => match state.field {
                EditField::Title => "edit title  tab body  esc done",
                EditField::Body => "edit body  tab title  esc done",
            },
            Mode::Command => "command mode",
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
            mode,
            Style::default().bg(Color::DarkGray).fg(Color::White),
        );
    }

    fn render_command_bar(&self, area: Rect, buf: &mut Buffer) {
        if !matches!(self.mode, Mode::Command) {
            return;
        }
        let width = min(area.width.saturating_sub(4), 60);
        let rect = Rect::new(area.x + 2, area.y + area.height.saturating_sub(3), width, 3);
        Clear.render(rect, buf);
        Block::default()
            .borders(Borders::ALL)
            .title(" Command ")
            .render(rect, buf);
        let inner = inset(rect, 1);
        if inner.width > 0 {
            buf.set_string(
                inner.x,
                inner.y,
                format!(":{}", self.command_buffer),
                Style::default(),
            );
        }
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
        if let Some(selected) = self.selected {
            let note = &mut self.notes[selected];
            note.x += dx * repeat;
            note.y += dy * repeat;
            self.dirty = true;
            self.status = format!("moved [{}] by {},{}", note.key, dx * repeat, dy * repeat);
        } else {
            self.camera_x += (dx * 4.0 * repeat) / self.zoom;
            self.camera_y += (dy * 2.5 * repeat) / self.zoom;
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
            title: format!("Node {}", key),
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
            title_cursor: self.notes[index].title.len(),
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

    fn begin_relation(&mut self, kind: RelationType) {
        if self.selected.is_none() {
            self.status = "select a source node first".into();
            return;
        }
        self.mode = Mode::AwaitRelationPrefix(kind);
        self.status = format!("{} relation: press f then target key", kind.label());
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

    fn node_by_key(&self, key: char) -> Option<&Note> {
        self.notes.iter().find(|note| note.key == key)
    }

    fn open_selected_in_editor(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    ) -> Result<()> {
        let Some(selected) = self.selected else {
            self.status = "select a node first".into();
            return Ok(());
        };

        let path = self.ensure_node_file(selected)?;
        self.sync_note_to_file(selected, &path)?;
        self.run_external_command(
            terminal,
            &format!("{} {}", self.editor_command, shell_quote(&path)),
        )?;
        self.sync_note_from_file(selected, &path)?;
        self.status = format!("opened {}", path.display());
        Ok(())
    }

    fn ensure_node_file(&mut self, index: usize) -> Result<PathBuf> {
        let note = &mut self.notes[index];
        let path = if let Some(path) = &note.file_path {
            path.clone()
        } else {
            let generated = PathBuf::from(format!("nodes/{}.md", slugify(&note.title)));
            note.file_path = Some(generated.clone());
            generated
        };
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        Ok(path)
    }

    fn sync_note_to_file(&self, index: usize, path: &Path) -> Result<()> {
        let note = &self.notes[index];
        let content = format!("{}\n\n{}", note.title, note.body);
        fs::write(path, content).with_context(|| format!("failed to write {}", path.display()))
    }

    fn sync_note_from_file(&mut self, index: usize, path: &Path) -> Result<()> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let mut parts = content.splitn(2, "\n\n");
        let title = parts.next().unwrap_or("").trim_end().to_string();
        let body = parts.next().unwrap_or("").to_string();
        let note = &mut self.notes[index];
        if !title.is_empty() {
            note.title = title;
        }
        note.body = body;
        note.file_path = Some(path.to_path_buf());
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
        if !status.success() {
            return Err(anyhow!("external command failed: {command}"));
        }
        Ok(())
    }

    fn save_loaded(&mut self) -> Result<()> {
        let Some(path) = self.loaded_path.clone() else {
            bail!("no loaded file; start with a .json or .mmd path");
        };
        self.save_to_path(&path)?;
        self.status = format!("saved {}", path.display());
        Ok(())
    }

    fn save_to_path(&mut self, path: &Path) -> Result<()> {
        match path.extension().and_then(|ext| ext.to_str()) {
            Some("json") => {
                let graph = GraphFile {
                    notes: self.notes.clone(),
                    relations: self.relations.clone(),
                    editor_command: Some(self.editor_command.clone()),
                };
                fs::write(path, serde_json::to_string_pretty(&graph)?)
                    .with_context(|| format!("failed to write {}", path.display()))?;
            }
            Some("mmd") => {
                fs::write(path, self.to_mermaid())
                    .with_context(|| format!("failed to write {}", path.display()))?;
            }
            _ => bail!("unsupported save format for {}", path.display()),
        }
        self.dirty = false;
        Ok(())
    }

    fn export_mermaid(&mut self) -> Result<PathBuf> {
        let path = if let Some(loaded) = &self.loaded_path {
            loaded.with_extension("mmd")
        } else {
            PathBuf::from("graph.mmd")
        };
        fs::write(&path, self.to_mermaid())
            .with_context(|| format!("failed to write {}", path.display()))?;
        Ok(path)
    }

    fn to_mermaid(&self) -> String {
        let mut out = String::from("graph LR\n");
        for note in &self.notes {
            out.push_str(&format!(
                "    {}[\"{}\"]\n",
                note.key,
                escape_mermaid(&note.title.replace('\n', " "))
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

    fn quit_requested(&mut self, force: bool) -> Result<()> {
        if self.dirty && !force {
            bail!("unsaved changes; use :q! to force or :w to save");
        }
        self.should_quit = true;
        Ok(())
    }

    fn next_key(&self) -> Option<char> {
        let used: Vec<char> = self.notes.iter().map(|note| note.key).collect();
        ('a'..='z')
            .chain('0'..='9')
            .find(|candidate| !used.contains(candidate))
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

        if let Some((left, title)) = trimmed.split_once("[\"") {
            let Some(title) = title.strip_suffix("\"]") else {
                continue;
            };
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

fn shell_quote(path: &Path) -> String {
    let raw = path.display().to_string();
    format!("'{}'", raw.replace('\'', "'\\''"))
}

fn move_cursor(state: &mut EditState, delta: isize, len: usize) {
    let cursor = match state.field {
        EditField::Title => &mut state.title_cursor,
        EditField::Body => &mut state.body_cursor,
    };
    let next = (*cursor as isize + delta).clamp(0, len as isize);
    *cursor = next as usize;
}

fn slugify(title: &str) -> String {
    let mut out = String::new();
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

fn slugify_key(key: char) -> String {
    key.to_string()
}

fn escape_mermaid(text: &str) -> String {
    text.replace('"', "\\\"")
}
