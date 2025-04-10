mod ansi;

use ansi::{AnsiCode, AnsiParser};
use ansi_colours;
use async_std::io::{stdout, WriteExt};
use futures::{SinkExt, StreamExt};
use iced::futures::Stream;
use iced::widget::{button, column, container, text, Column};
use iced::{self, *};
use keyboard::key::Named;
use keyboard::{on_key_press, Key, Modifiers};
use libc::winsize;
use mouse::ScrollDelta;
use nix::pty::{forkpty, ForkptyResult};
use nix::sys::termios::Termios;
use std::fmt::Display;
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::process::Command;
use std::str::FromStr;
use std::thread::sleep;
use std::time::Duration;
use tokio::io::AsyncReadExt as _;
use tokio::sync::mpsc::channel;
use widget::container::{background, dark, Style};
use widget::{row, scrollable, Row, Scrollable};

const ROWS: u16 = 37;
const COLS: u16 = 100;

const MONO: Font = Font {
    family: font::Family::Monospace,
    weight: font::Weight::Normal,
    stretch: font::Stretch::Normal,
    style: font::Style::Normal,
};

pub enum Event {
    Start(File),
    Done,
}

#[derive(Debug)]
pub enum Content {
    Text(String),
    Bytes(Vec<u8>),
    Key(Named),
    Sigint,
}

#[derive(Debug)]
pub enum Message {
    Init(File),
    Write(Content),
    Output(Vec<Output>),
    WindowResized(Size),
}

impl From<&str> for Content {
    fn from(s: &str) -> Self {
        Self::Text(s.to_string())
    }
}

impl From<Named> for Content {
    fn from(named: Named) -> Self {
        Self::Key(named)
    }
}

impl Message {
    fn write<C: Into<Content>>(c: C) -> Self {
        Self::Write(c.into())
    }

    fn bytes<V: Into<Vec<u8>>>(v: V) -> Self {
        Self::Write(Content::Bytes(v.into()))
    }

    fn named(named: Named) -> Self {
        Self::Write(named.into())
    }
}

#[derive(Debug)]
pub enum Output {
    Ansi(AnsiCode),
    Bytes(Vec<u8>),
}

impl Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bytes(b) => write!(f, "{}", String::from_utf8_lossy(b)),
            Self::Ansi(ac) => write!(f, "{:?}", ac),
        }
    }
}

impl From<ansi::Output<'_>> for Output {
    fn from(value: ansi::Output<'_>) -> Self {
        match value {
            ansi::Output::Bytes(b) => Self::Bytes(b.to_vec()),
            ansi::Output::Escape(ac) => Self::Ansi(ac),
        }
    }
}

#[derive(Default, Debug)]
pub struct State {
    grid: Grid,
    brush: Brush,
}

#[derive(Debug, Copy, Clone)]
pub enum TermColor {
    Rgb(u8, u8, u8),
    Ansi(u8),
}

impl TermColor {
    pub fn default_fg() -> Self {
        Self::white()
    }

    pub fn default_bg() -> Self {
        Self::dark()
    }

    pub fn white() -> Self {
        Self::Rgb(255, 255, 255)
    }

    pub fn black() -> Self {
        Self::Rgb(0, 0, 0)
    }

    pub fn dark() -> Self {
        Self::Rgb(30, 30, 30)
    }

    pub fn red() -> Self {
        Self::Rgb(255, 0, 0)
    }
}

#[derive(Debug)]
pub struct Brush {
    fg_color: TermColor,
    bg_color: TermColor,
    pos: (usize, usize),
}

impl Default for Brush {
    fn default() -> Self {
        Self {
            pos: (1, 1),
            bg_color: TermColor::default_bg(),
            fg_color: TermColor::default_fg(),
        }
    }
}

impl Brush {
    pub fn reset_color(&mut self) {
        self.fg_color = TermColor::default_fg();
        self.bg_color = TermColor::default_bg();
    }
}

#[derive(Default, Debug)]
pub struct Grid {
    rows: Vec<GridRow>,
}

#[derive(Default, Debug)]
pub struct GridRow {
    cells: Vec<Cell>,
}

#[derive(Debug)]
pub struct Cell {
    pub fg_color: TermColor,
    pub bg_color: TermColor,
    pub c: char,
}

impl Default for Cell {
    fn default() -> Self {
        Self::empty()
    }
}

impl Cell {
    fn empty() -> Self {
        Self {
            c: ' ',
            fg_color: TermColor::default_fg(),
            bg_color: TermColor::default_bg(),
        }
    }
}

impl Grid {
    pub fn erase_line(&mut self, brush: &Brush) {
        let row = self.get_or_insert(brush.pos.1);
        let x = brush.pos.0 - 1;

        while row.cells.len() > x {
            row.cells.pop();
        }
    }

    pub fn paint(&mut self, brush: &Brush, char: char) {
        let Brush {
            pos: (x, y),
            bg_color,
            fg_color,
        } = brush;

        let cell = self.get_or_insert(*y).get_or_insert(*x);
        cell.fg_color = *fg_color;
        cell.bg_color = *bg_color;
        cell.c = char;
    }

    fn get_or_insert(&mut self, y: usize) -> &mut GridRow {
        let y = y - 1;
        while y >= self.rows.len() {
            self.rows.push(GridRow::default());
        }

        &mut self.rows[y]
    }

    fn erase_display_from(&mut self, brush: &Brush) {
        let (x, y) = brush.pos;
        for i in 0..ROWS as usize {
            let row = self.get_or_insert(y + i);
            for cell in row.cells.iter_mut() {
                cell.c = ' ';
                cell.fg_color = TermColor::default_fg();
                cell.bg_color = TermColor::default_bg();
            }
        }
    }

    fn erase_display_preserve_cursor(&mut self, brush: &Brush) {}
}

impl GridRow {
    fn get_or_insert(&mut self, x: usize) -> &mut Cell {
        let x = x - 1;
        while x >= self.cells.len() {
            self.cells.push(Cell::default());
        }

        &mut self.cells[x]
    }
}

impl State {
    fn window(&self, height: usize) -> &[GridRow] {
        let l = self.grid.rows.len();
        if height > l {
            &self.grid.rows[..]
        } else {
            &self.grid.rows[l - height..]
        }
    }

    fn text(&self) -> String {
        let mut text = String::new();

        for row in self.grid.rows.iter() {
            for cell in row.cells.iter() {
                text.push(cell.c);
            }
            text.push('\n');
        }

        text
    }
}

impl From<&Cell> for Element<'_, Message> {
    fn from(cell: &Cell) -> Self {
        let bg_color = Color::from(&cell.bg_color);
        let fg_color = Color::from(&cell.fg_color);
        container(text(cell.c.to_string()).font(MONO).color(fg_color))
            .style(move |_| background(Background::Color(bg_color)))
            .into()
    }
}

impl From<&TermColor> for Color {
    fn from(tc: &TermColor) -> Self {
        match *tc {
            TermColor::Rgb(r, g, b) => Color {
                r: r as f32 / 255.0,
                g: g as f32 / 255.0,
                b: b as f32 / 255.0,
                a: 1.0,
            },
            TermColor::Ansi(_) => todo!(),
        }
    }
}

#[derive(Default, Debug)]
pub struct Screen {
    handle: Option<File>,
    contents: Vec<String>,
    state: State,
    curr_size: Size,
}

impl Screen {
    pub fn new() -> Self {
        Self {
            ..Default::default()
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let window = self.state.window(ROWS as usize);

        let mut lines: Vec<Element<'_, Message>> = vec![];
        for line in window.iter() {
            let mut column: Vec<Element<'_, Message>> = vec![];
            for cell in line.cells.iter() {
                column.push(Element::from(cell));
            }
            let col: Element<'_, Message> = Row::with_children(column).into();
            lines.push(col);
        }

        let rows = Column::with_children(lines);
        let bg_color = Color::from(&TermColor::dark());
        let style = Style::default().background(Background::Color(bg_color));
        container(rows)
            .height(1024)
            .width(2048)
            .style(move |_| style)
            .into()
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::Init(handle) => self.handle = Some(handle),
            Message::Output(s) => self.handle_output(s),
            Message::Write(c) => {
                let Some(handle) = self.handle.as_mut() else {
                    return;
                };

                match c {
                    Content::Text(s) => handle.write_all(s.as_bytes()).unwrap(),
                    Content::Bytes(b) => handle.write_all(b.as_slice()).unwrap(),
                    Content::Sigint => handle.write_all(b"\x03").unwrap(),
                    Content::Key(named) => match named {
                        Named::Enter => handle.write_all(b"\n").unwrap(),
                        Named::Space => handle.write_all(b" ").unwrap(),
                        Named::Backspace => handle.write_all(b"\x7F").unwrap(),
                        Named::Escape => handle.write_all(b"\x1b").unwrap(),
                        _named => {}
                    },
                };
            }
            Message::WindowResized(size) => {
                self.curr_size = size;
            }
        };
    }

    pub fn handle_bytes(&mut self, bytes: Vec<u8>) {
        match bytes.as_slice() {
            b"\x07" => { // according to chatgpt this is when there is nothing else to backspace
                 // to, some terminals emit a sound (idk)
            }
            b"\x08" => { // according to chatgpt this is to move the cursor to the left after a
                 // backspace??? not sure about that
            }
            b"\x08\x1b\x5b\x4b" => {
                // backspace
                let _ = self.contents.last_mut().and_then(|l| l.pop());
            }
            _ => {
                let Ok(parsed) = String::from_utf8(bytes) else {
                    eprintln!("failed to parse");
                    return;
                };

                for char in parsed.chars() {
                    match char {
                        '\n' => {
                            self.state.brush.pos.1 += 1;
                        }
                        '\r' => {
                            self.state.brush.pos.0 = 1;
                        }
                        '\t' => {
                            self.state.brush.pos.0 += 4;
                        }
                        '\u{1b}' => {}
                        '\u{8}' => {
                            self.state.brush.pos.0 -= 1;
                        }
                        _ => {
                            self.state.grid.paint(&self.state.brush, char);
                            self.state.brush.pos.0 += 1;
                        }
                    }
                }
            }
        };
    }

    pub fn handle_ansi(&mut self, ac: AnsiCode) {
        use AnsiCode::*;

        match ac {
            EraseLine => {
                self.state.grid.erase_line(&self.state.brush);
            }
            EraseDisplay => {
                // deletes all text from the cursor position to the end of the screen

                //self.state.grid.erase_display_from(&self.state.brush);
            }
            EraseAllDisplay => {
                // deletes all text in the screen and preserves cursor position

                self.state.grid.erase_display_from(&self.state.brush);
            }
            CursorSave => {}
            SetGraphicsMode(1, [0, _, _, _, _]) => {
                self.state.brush.reset_color();
            }
            SetGraphicsMode(1, [39, _, _, _, _]) => {
                self.state.brush.fg_color = TermColor::default_fg();
            }

            SetGraphicsMode(1, [49, _, _, _, _]) => {
                self.state.brush.bg_color = TermColor::default_bg();
            }
            SetGraphicsMode(3, [38, 5, id, _, _]) => {
                let (r, g, b) = ansi_colours::rgb_from_ansi256(id);
                self.state.brush.fg_color = TermColor::Rgb(r, g, b);
            }
            SetGraphicsMode(3, [48, 5, id, _, _]) => {
                let (r, g, b) = ansi_colours::rgb_from_ansi256(id);
                self.state.brush.bg_color = TermColor::Rgb(r, g, b);
            }
            SetGraphicsMode(5, [38, 2, r, g, b]) => {
                self.state.brush.fg_color = TermColor::Rgb(r, g, b);
            }
            SetGraphicsMode(5, [48, 2, r, g, b]) => {
                self.state.brush.bg_color = TermColor::Rgb(r, g, b);
            }
            _ => {}
        }
    }

    pub fn handle_output(&mut self, outputs: Vec<Output>) {
        for op in outputs.iter() {
            print!("{}, ", op);
        }
        for output in outputs {
            match output {
                Output::Bytes(b) => self.handle_bytes(b),
                Output::Ansi(ac) => self.handle_ansi(ac),
            }
        }
    }
}

fn handle_key(key: Key, mods: Modifiers) -> Option<Message> {
    use iced::keyboard::Key as IKey;
    use Content::*;
    use Message::*;

    match key {
        IKey::Character(c) if mods.control() && c.as_str() == "c" => Some(Write(Sigint)),
        IKey::Character(c) if mods.shift() && c.as_str() == "7" => Some(Message::write("&")),
        IKey::Character(c) if mods.shift() && c.as_str() == "\\" => Some(Message::write("|")),
        IKey::Character(c) if mods.shift() && c.as_str() == "-" => Some(Message::write("_")),
        IKey::Character(c) if mods.shift() && c.as_str() == ";" => Some(Message::write(":")),
        IKey::Character(c) if mods.shift() && c.as_str() == "1" => Some(Message::write("!")),
        IKey::Character(c) => Some(Write(Text(c.to_string()))),
        IKey::Named(named) => Some(Message::named(named)),
        _ => None,
    }
}

fn start_slave_process() {
    let _ = Command::new("/bin/zsh").exec();
    std::process::exit(0)
}

fn pcomms() -> impl Stream<Item = Message> {
    stream::channel(100, |mut output| async move {
        let winsize = winsize {
            ws_row: 50,
            ws_col: 100,
            ws_xpixel: 1024,
            ws_ypixel: 2048,
        };

        let result = unsafe { forkpty(&winsize, None).unwrap() };

        let master = match result {
            ForkptyResult::Parent { master, .. } => master,
            ForkptyResult::Child => {
                start_slave_process();
                std::process::exit(0);
            }
        };

        let (tx, mut rx) = channel::<Vec<Output>>(100);
        let whandle: File = master.into();
        let mut rhandle = tokio::fs::File::from(whandle.try_clone().unwrap());

        output.send(Message::Init(whandle)).await.unwrap();
        async_std::task::spawn(async move {
            let mut buf = [0u8; 1024];
            loop {
                let n = rhandle.read(&mut buf).await.unwrap();
                let items = AnsiParser::new(&buf[..n])
                    .map(Output::from)
                    .collect::<Vec<Output>>();

                tx.send(items).await.unwrap();
            }
        });

        loop {
            if let Some(msg) = rx.recv().await {
                output.send(Message::Output(msg)).await.unwrap();
                output.flush().await.unwrap();
            }
        }
    })
}

fn subscription(_s: &Screen) -> Subscription<Message> {
    use event::Event as AppEvent;

    fn keyboard_sub() -> Subscription<Message> {
        on_key_press(handle_key)
    }

    fn process_comm_sub() -> Subscription<Message> {
        Subscription::run(pcomms)
    }

    fn window_resize() -> Subscription<Message> {
        event::listen_with(|event, _status, _id| match event {
            AppEvent::Window(window::Event::Resized(size)) => Some(Message::WindowResized(size)),
            _ => None,
        })
    }

    fn mouse_sub() -> Subscription<Message> {
        fn handle_delta(delta: ScrollDelta) -> Option<Message> {
            match delta {
                ScrollDelta::Lines { x, y } if y < 0.0 => Some(Message::bytes(b"\x1b[S")),
                ScrollDelta::Lines { x, y } if y > 0.0 => Some(Message::bytes(b"\x1b[T")),
                ScrollDelta::Pixels { x, y } => Some(Message::bytes(b"\x1b[T")),
                _ => None,
            }
        }

        event::listen_with(|e, _status, _id| match e {
            AppEvent::Mouse(mouse::Event::WheelScrolled { delta }) => handle_delta(delta),
            _ => None,
        })
    }
    Subscription::batch([process_comm_sub(), keyboard_sub(), mouse_sub()])
}

#[tokio::main]
pub async fn main() -> iced::Result {
    iced::application("A toy terminal emulator", Screen::update, Screen::view)
        .subscription(subscription)
        .run()
}
