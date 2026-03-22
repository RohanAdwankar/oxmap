use std::cmp::{max, min};
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
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
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

fn main() -> Result<()> {
    let mut terminal = setup_terminal()?;
    let app_result = run_app(&mut terminal);
    restore_terminal(&mut terminal)?;
    app_result
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<std::io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>) -> Result<()> {
    let mut app = App::demo();

    loop {
        terminal.draw(|frame| app.render(frame.area(), frame.buffer_mut()))?;

        if !event::poll(Duration::from_millis(50))? {
            continue;
        }

        if let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break,
                KeyCode::Left | KeyCode::Char('h') => app.pan(-8.0, 0.0),
                KeyCode::Right | KeyCode::Char('l') => app.pan(8.0, 0.0),
                KeyCode::Up | KeyCode::Char('k') => app.pan(0.0, -5.0),
                KeyCode::Down | KeyCode::Char('j') => app.pan(0.0, 5.0),
                KeyCode::Char('+') | KeyCode::Char('=') => app.zoom(1.2),
                KeyCode::Char('-') | KeyCode::Char('_') => app.zoom(1.0 / 1.2),
                KeyCode::Char('0') => app.reset_view(),
                _ => {}
            }
        }
    }

    Ok(())
}

#[derive(Clone, Copy)]
struct Note {
    title: &'static str,
    body: &'static str,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: Color,
}

struct App {
    notes: Vec<Note>,
    camera_x: f32,
    camera_y: f32,
    zoom: f32,
}

impl App {
    fn demo() -> Self {
        Self {
            notes: vec![
                Note {
                    title: "Inbox",
                    body: "quick capture\nmeeting notes",
                    x: -30.0,
                    y: -8.0,
                    w: 20.0,
                    h: 8.0,
                    color: Color::Cyan,
                },
                Note {
                    title: "Design",
                    body: "progressive detail\nzoomed canvas",
                    x: -2.0,
                    y: -12.0,
                    w: 24.0,
                    h: 9.0,
                    color: Color::Yellow,
                },
                Note {
                    title: "API",
                    body: "cluster nearby cards\nbefore text vanishes",
                    x: 29.0,
                    y: -4.0,
                    w: 25.0,
                    h: 8.0,
                    color: Color::Green,
                },
                Note {
                    title: "Roadmap",
                    body: "terminal map\nobsidian-style overview",
                    x: -18.0,
                    y: 10.0,
                    w: 22.0,
                    h: 8.0,
                    color: Color::Magenta,
                },
                Note {
                    title: "Ideas",
                    body: "semantic zoom\n1-char glyphs when far away",
                    x: 15.0,
                    y: 13.0,
                    w: 26.0,
                    h: 9.0,
                    color: Color::Blue,
                },
                Note {
                    title: "Links",
                    body: "future work:\nedge routing",
                    x: 52.0,
                    y: 16.0,
                    w: 18.0,
                    h: 7.0,
                    color: Color::Red,
                },
            ],
            camera_x: 10.0,
            camera_y: 5.0,
            zoom: 1.4,
        }
    }

    fn pan(&mut self, dx: f32, dy: f32) {
        self.camera_x += dx / self.zoom;
        self.camera_y += dy / self.zoom;
    }

    fn zoom(&mut self, factor: f32) {
        self.zoom = (self.zoom * factor).clamp(0.08, 4.0);
    }

    fn reset_view(&mut self) {
        self.camera_x = 10.0;
        self.camera_y = 5.0;
        self.zoom = 1.4;
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        Block::default()
            .borders(Borders::ALL)
            .border_set(border::THICK)
            .title(" Note Graph ")
            .title_bottom(Line::from(" arrows/hjkl pan  +/- zoom  0 reset  q quit "))
            .render(area, buf);

        let canvas = inset(area, 1);
        if canvas.width < 4 || canvas.height < 4 {
            return;
        }

        fill_area(buf, canvas, ' ', Style::default().bg(Color::Black));
        self.render_grid(canvas, buf);
        self.render_notes(canvas, buf);
        self.render_status(canvas, buf);
    }

    fn render_grid(&self, area: Rect, buf: &mut Buffer) {
        let world_step = 10.0;
        let screen_step = self.zoom * world_step;
        if screen_step < 4.0 {
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

    fn render_notes(&self, area: Rect, buf: &mut Buffer) {
        let mut clusters = vec![Cluster::default(); area.width as usize * area.height as usize];

        for note in &self.notes {
            let projected = self.project_rect(*note, area);
            if projected.w >= 8 && projected.h >= 5 {
                self.render_note_box(note, projected, area, buf);
            } else {
                self.add_cluster(note, area, &mut clusters);
            }
        }

        self.render_clusters(area, buf, &clusters);
    }

    fn project_rect(&self, note: Note, area: Rect) -> ProjectedRect {
        let x = ((note.x - self.camera_x) * self.zoom + area.width as f32 / 2.0).round() as i32;
        let y = ((note.y - self.camera_y) * self.zoom + area.height as f32 / 2.0).round() as i32;
        let w = max(1, (note.w * self.zoom).round() as i32);
        let h = max(1, (note.h * self.zoom).round() as i32);
        ProjectedRect { x, y, w, h }
    }

    fn render_note_box(&self, note: &Note, projected: ProjectedRect, area: Rect, buf: &mut Buffer) {
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

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(note.color))
            .title(Line::styled(
                format!(" {} ", note.title),
                Style::default().fg(note.color).add_modifier(Modifier::BOLD),
            ));
        block.render(rect, buf);

        let inner = inset(rect, 1);
        if inner.width > 0 && inner.height > 0 {
            Paragraph::new(note.body)
                .style(Style::default().fg(Color::White))
                .render(inner, buf);
        }
    }

    fn add_cluster(&self, note: &Note, area: Rect, clusters: &mut [Cluster]) {
        let cx = ((note.x + note.w / 2.0 - self.camera_x) * self.zoom + area.width as f32 / 2.0)
            .round() as i32;
        let cy = ((note.y + note.h / 2.0 - self.camera_y) * self.zoom + area.height as f32 / 2.0)
            .round() as i32;
        if cx < 0 || cy < 0 || cx >= area.width as i32 || cy >= area.height as i32 {
            return;
        }

        let idx = cy as usize * area.width as usize + cx as usize;
        let cluster = &mut clusters[idx];
        cluster.count += 1;
        cluster.color = merge_color(cluster.color, note.color);
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
                let style = Style::default()
                    .fg(cluster.color.unwrap_or(Color::Gray))
                    .add_modifier(if cluster.count >= 3 {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    });
                let cell = buf
                    .cell_mut((area.x + x as u16, area.y + y as u16))
                    .expect("cluster cell in bounds");
                cell.set_char(glyph).set_style(style);
            }
        }
    }

    fn render_status(&self, area: Rect, buf: &mut Buffer) {
        let status = format!(
            "zoom {:.2}x  camera ({:.0}, {:.0})",
            self.zoom, self.camera_x, self.camera_y
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
}

fn merge_color(existing: Option<Color>, incoming: Color) -> Option<Color> {
    existing.or(Some(incoming))
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
            let cell = buf.cell_mut((x, y)).expect("fill cell in bounds");
            cell.set_char(ch).set_style(style);
        }
    }
}
