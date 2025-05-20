// Importing necessary modules from the Crossterm crate
use crossterm::{
    cursor, // For controlling the cursor
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent}, // For handling keyboard/mouse events
    execute, // Macro to execute a batch of terminal commands
    style::Print, // To print styled or plain text
    terminal::{self, Clear, ClearType, disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen, size}, // Terminal control
};
use std::{io::{self, Write}}; // Standard IO utilities 

// Define a struct `Editor` that holds editor state
struct Editor {
    cursor_x: usize, // Cursor's column position
    cursor_y: usize,  // Cursor's row position
    screen_rows: u16, // Number of rows in the visible screen
    screen_cols: u16, // Number of columns in the visible screen
    rows: Vec<String>, // Stores lines of text in the editor
}

impl Editor {
    // Constructor: Initializes a new Editor with terminal size and one empty line
    fn new() -> std::io::Result<Self> {
        let (cols, rows) = size()?; // Get terminal width and height
        Ok(Self {
            cursor_x: 0,
            cursor_y: 0,
            screen_rows: rows,
            screen_cols: cols,
            rows: vec![String::new()], // Start with one empty line
        })
    }

    // Draw all rows of the editor to the terminal
    fn draw_rows(&self, stdout: &mut io::Stdout) -> std::io::Result<()> {
        for i in 0..self.screen_rows as usize {
            execute!(stdout, cursor::MoveTo(0, i as u16))?; // Move to the beginning of each line
            if i < self.rows.len() {
                let line = &self.rows[i];
                if line.len() > self.screen_cols as usize {
                    // Print only as much as fits on screen
                    execute!(stdout, Print(&line[..self.screen_cols as usize]))?;
                } else {
                    execute!(stdout, Print(line))?;
                }
            } else {
                execute!(stdout, Print("~"))?; // Placeholder for unused lines
            }
        }
        Ok(())
    }

    // Refresh the entire screen: clear and redraw
    fn refresh_screen(&self, stdout: &mut io::Stdout) -> std::io::Result<()> {
        execute!(
            stdout,
            cursor::Hide,// Hide cursor during drawing
            cursor::MoveTo(0, 0), // Move cursor to top-left
            Clear(ClearType::All) // Clear the entire terminal
        )?;
        self.draw_rows(stdout)?;  // Draw current editor content
        // restrict cursor within visible screen
        let cx = self.cursor_x.min(self.screen_cols as usize - 1) as u16;
        let cy = self.cursor_y.min(self.screen_rows as usize - 1) as u16;        
        execute!(
            stdout,
            cursor::MoveTo(cx, cy),// Move cursor to correct position
            cursor::Show // Show the cursor
        )?;
        stdout.flush()?; // Flush all output to terminal
        Ok(())
    }

    // Handle keypress events, return true if 'q' is pressed to quit
    fn process_keypress(&mut self, event: KeyEvent) -> std::io::Result<bool> {
        // Only process if there is an event ready
        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(KeyEvent { code, .. }) = event::read()? {
                match code {
                    KeyCode::Char('q') => return Ok(true), // Quit editor on 'q'
                    // Insert printable character at current cursor position
                    KeyCode::Char(c) => {
                        if self.cursor_y < self.rows.len() {
                            let line = &mut self.rows[self.cursor_y];
                            if self.cursor_x <= line.len() {
                                line.insert(self.cursor_x, c);
                                self.cursor_x += 1;
                            }
                        }
                    }
                    // Handle backspace: remove char or merge lines
                    KeyCode::Backspace => {
                        if self.cursor_y < self.rows.len() {
                            if self.cursor_x > 0 {
                                let line = &mut self.rows[self.cursor_y];
                                line.remove(self.cursor_x - 1);
                                self.cursor_x -= 1;
                            } else if self.cursor_y > 0 {
                                let current_line = self.rows.remove(self.cursor_y);
                                self.cursor_y -= 1;
                                self.cursor_x = self.rows[self.cursor_y].len();
                                self.rows[self.cursor_y].push_str(&current_line);
                            }
                        }
                    }
                    //Usual eneter key function
                    KeyCode::Enter => {
                        //splits the line from current cursor adn moves the right part below
                        if self.cursor_y < self.rows.len() {
                            let line = &mut self.rows[self.cursor_y];
                            let new_line = line.split_off(self.cursor_x);
                            self.cursor_y += 1;
                            self.cursor_x = 0;
                            self.rows.insert(self.cursor_y, new_line);
                        }
                    }
                    // Move cursor left
                    KeyCode::Left => {
                        if self.cursor_x > 0 {
                            self.cursor_x -= 1;
                        } else if self.cursor_y > 0 {
                            self.cursor_y -= 1;
                            self.cursor_x = self.rows[self.cursor_y].len();
                        }
                    }
                    // Move cursor right
                    KeyCode::Right => {
                        if self.cursor_y < self.rows.len() {
                            if self.cursor_x < self.rows[self.cursor_y].len() {
                                self.cursor_x += 1;
                            } else if self.cursor_y + 1 < self.rows.len() {
                                self.cursor_y += 1;
                                self.cursor_x = 0;
                            }
                        }
                    }
                    // Move cursor up
                    KeyCode::Up => {
                        if self.cursor_y > 0 {
                            self.cursor_y -= 1;
                            self.cursor_x = self.cursor_x.min(self.rows[self.cursor_y].len());
                        }
                    }
                    // Move cursor down
                    KeyCode::Down => {
                        if self.cursor_y + 1 < self.rows.len() {
                            self.cursor_y += 1;
                            self.cursor_x = self.cursor_x.min(self.rows[self.cursor_y].len());
                        }
                    }
                    _ => {} // Ignore other keys
                }
            }
        }
        Ok(false)
    }
}

// Entry point for the program
fn main() -> std::io::Result<()> {
    enable_raw_mode()?; // Enable raw mode
    let mut stdout = io::stdout();
    // Switch to alternate screen & enable mouse capture
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        cursor::SetCursorStyle::BlinkingBar,
    )?;
    //Initialization
    let mut editor = Editor::new()?;
    // Main input loop
    loop {
        editor.refresh_screen(&mut stdout)?; // Redraw screen
        // Handling inputs
        if event::poll(std::time::Duration::from_millis(500))? {
            if let Event::Key(key_event) = event::read()? {
                if editor.process_keypress(key_event)? {
                    break; // Exit loop on 'q'
                }
            }
        }
    }
    //restore normal terminal mode
    disable_raw_mode()?;
    execute!(
        stdout,
        LeaveAlternateScreen,
        DisableMouseCapture,
        cursor::SetCursorStyle::DefaultUserShape,
        cursor::Show
    )?;
    Ok(())
}
