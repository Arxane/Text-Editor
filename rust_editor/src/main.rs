// Importing necessary modules from the Crossterm for Cross-terminal compatibility
use crossterm::{
    cursor, // For controlling the cursor
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers, KeyEventKind}, // For handling keyboard/mouse events
    execute, // Macro to execute a batch of terminal commands
    style::{Print, Color, Stylize}, // To print styled or plain text
    terminal::{self, Clear, ClearType, disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen, size}, // Terminal control
};
use std::{env, fs, io::{self, Write}, path::Path, result, thread::current, time::{Duration,Instant}}; // Standard IO utilities 
use std::fs::OpenOptions;

// Define a struct `Editor` that holds editor state
struct Editor {
    cursor_x: usize, // Cursor's column position
    cursor_y: usize,  // Cursor's row position
    screen_rows: u16, // Number of rows in the visible screen
    screen_cols: u16, // Number of columns in the visible screen
    rows: Vec<String>, // Stores lines of text in the editor
    filename: Option<String>, //Optional filename if its loaded
    dirty: bool, //tracks whether if file is modified
    last_key_time: Instant, //Timestamp of last key press
    last_key: Option<KeyEvent>, //last key event, used for debouncing repeated keypresses
    col_offset: usize, //to check for test more than columns
    undo_stack: Vec<EditorState>,
    redo_stack: Vec<EditorState>,
    search_mode: bool,
    search_query: String,
    search_results: Vec<(usize, usize)>, // (row, col)
    current_match: usize,
}

#[derive(Clone)]
struct EditorState{
    buffer: Vec<String>,
    cursor_x: usize,
    cursor_y: usize,
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
            rows: vec![String::new()],// Start with one empty line
            filename: None, 
            dirty: false,
            last_key_time: Instant::now(), //Initialize debounce timer
            last_key: None, //No previous key pressed
            col_offset: 0,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            search_mode: false,
            search_query: String::new(),
            search_results: Vec::new(),
            current_match: 0,
        })
    }

    fn open(&mut self, filename: &str)->std::io::Result<()>{ //error if not able to read therefore result used
        let contents = fs::read_to_string(filename)?; // read entire file to a string
        self.rows = contents.lines().map(|l| l.to_string()).collect(); //assign line to respective rows
        self.filename = Some(filename.to_string());
        self.dirty = false; //file is just opened, no unsaved changes
        Ok(())
    }

    fn save(&mut self)-> std::io::Result<()>{
        if let Some(name) = &self.filename {
            let data  = self.rows.join("\n"); //join all lines with new line
            fs::write(name, data)?; // Added ? to handle potential errors
            self.dirty = false; //npot dirty as it has been just saved
        }
        Ok(())
    }

    // Draw all rows of the editor to the terminal
    fn draw_rows(&self, stdout: &mut io::Stdout) -> std::io::Result<()> {
        for i in 0..self.screen_rows as usize {
            execute!(stdout, cursor::MoveTo(0, i as u16))?; // Move to the beginning of each line
            if i < self.rows.len() {
                let line = &self.rows[i];
                execute!(stdout, cursor::MoveTo(0,i as u16))?;
                let visible = if self.col_offset < line.len() {
                    &line[self.col_offset..]
                } else {
                    ""
                };
                let screen_cols = self.screen_cols as usize;
                let mut display_line = String::new();

                if self.col_offset > 0 {
                    display_line.push('»');
                    //Make sure we only render more characters
                    display_line.push_str(&visible.chars().take(screen_cols - 1).collect::<String>());
                } else {
                    display_line.push_str(&visible.chars().take(screen_cols).collect::<String>());
                }

                // Apply syntax highlighting
                let tokens = self.highlight_line(&display_line);
                for (token, color) in tokens {
                    execute!(stdout, Print(token.with(color)))?;
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
        self.draw_status_bar(stdout)?; //draw status bar
        if self.search_mode {
            self.draw_search_prompt(stdout)?;
        }
        // restrict cursor within visible screen
        let cx = self.cursor_x.saturating_sub(self.col_offset) as u16;
        let cy = self.cursor_y.min(self.screen_rows as usize - 1) as u16;        
        execute!(
            stdout,
            cursor::MoveTo(cx, cy),// Move cursor to correct position
            cursor::Show // Show the cursor
        )?;
        stdout.flush()?; // Flush all output to terminal
        Ok(())
    }

    // Handle keypress events, return true if 'Alt+q' is pressed to quit
    fn process_keypress(&mut self, event: KeyEvent) -> bool {
        if event.kind != KeyEventKind::Press {
            return false; //handle only actual keypresses and ignore repeats or releases
        }
        
        // Simple debouncing: ignore if same key pressed within 50ms
        let now = Instant::now();
        if let Some(last_key) = self.last_key {
            if now.duration_since(self.last_key_time) < Duration::from_millis(50) 
                && last_key.code == event.code 
                && last_key.modifiers == event.modifiers {
                return false;
            }
        }
        
        self.last_key_time = now;
        self.last_key = Some(event);
        match event.code {
            KeyCode::Char('q') if event.modifiers.contains(KeyModifiers::ALT) => return true, // Quit editor on Alt + q
            KeyCode::Char('s') if event.modifiers.contains(KeyModifiers::ALT) => {
                if let Err(e) = self.save() {
                    eprintln!("Failed to save file: {}", e);
                }
                self.dirty = false; // Mark as not dirty after save
            }
            KeyCode::Char('z') if event.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(prev) = self.undo_stack.pop() {
                    self.redo_stack.push(self.snapshot());
                    self.restore(prev);
                }
            }
            KeyCode::Char('x') if event.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(next) = self.redo_stack.pop() {
                    self.undo_stack.push(self.snapshot());
                    self.restore(next);
                }
            }
            KeyCode::Char(c) => {
                self.push_undo();
                if self.cursor_y < self.rows.len() {
                    let line = &mut self.rows[self.cursor_y];
                    if self.cursor_x <= line.len() {
                        line.insert(self.cursor_x, c);
                        self.cursor_x += 1;
                        self.dirty = true; // Mark as dirty when content changes
                    }
                }
            }
            KeyCode::Backspace => {
                self.push_undo();
                if self.cursor_y < self.rows.len() {
                    if self.cursor_x > 0 {
                        let line = &mut self.rows[self.cursor_y];
                        line.remove(self.cursor_x - 1);
                        self.cursor_x -= 1;
                        self.dirty = true; // Mark as dirty
                    } else if self.cursor_y > 0 {
                        let current_line = self.rows.remove(self.cursor_y);
                        self.cursor_y -= 1;
                        self.cursor_x = self.rows[self.cursor_y].len();
                        self.rows[self.cursor_y].push_str(&current_line);
                        self.dirty = true; // Mark as dirty
                    }
                }
            }
            KeyCode::Enter => {
                self.push_undo();
                if self.cursor_y < self.rows.len() {
                    let line = &mut self.rows[self.cursor_y];
                    let new_line = line.split_off(self.cursor_x);
                    self.cursor_y += 1;
                    self.cursor_x = 0;
                    self.rows.insert(self.cursor_y, new_line);
                    self.dirty = true; // Mark as dirty
                }
            }
            KeyCode::Left => {
                if self.cursor_x > 0 {
                    self.cursor_x -= 1;
                } else if self.cursor_y > 0 {
                    self.cursor_y -= 1;
                    self.cursor_x = self.rows[self.cursor_y].len();
                }
            }
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
            KeyCode::Up => {
                if self.cursor_y > 0 {
                    self.cursor_y -= 1;
                    self.cursor_x = self.cursor_x.min(self.rows[self.cursor_y].len());
                }
            }
            KeyCode::Down => {
                if self.cursor_y + 1 < self.rows.len() {
                    self.cursor_y += 1;
                    self.cursor_x = self.cursor_x.min(self.rows[self.cursor_y].len());
                }
            }
            

            _ => {}
        }
        let screen_cols = self.screen_cols as usize;

        if self.cursor_x < self.col_offset {
            self.col_offset = self.cursor_x;
        } else if self.cursor_x >= self.col_offset + screen_cols {
            self.col_offset = self.cursor_x - screen_cols + 1;
        }
        false
    }

    fn draw_status_bar(&self, stdout: &mut io::Stdout) -> std::io::Result<()> {
        use crossterm::style::{SetAttribute, Attribute, SetBackgroundColor, SetForegroundColor, Color};
        let file_name = self.filename.as_deref().unwrap_or("[No Name]");
        let status = if self.dirty {"[Modified]"} else {""};
        let info = format!("{} {}", file_name, status);

        let pos = format!("Ln {}, Col {}", self.cursor_y+1, self.cursor_x+1);
        let padding = (self.screen_cols as usize).saturating_sub(info.len()+pos.len());
        let status_line = format!("{}{}{}", info, " ".repeat(padding), pos);
        execute!(
            stdout,
            cursor::MoveTo(0, self.screen_rows - 1),
            SetBackgroundColor(Color::DarkGrey),
            SetForegroundColor(Color::White),
            SetAttribute(Attribute::Bold),
            Print(&status_line[..self.screen_cols as usize]),
            SetAttribute(Attribute::Reset),
            SetForegroundColor(Color::Reset),
            SetBackgroundColor(Color::Reset)
        )?;

        Ok(())
    }

    fn highlight_line(&self, line: &str)-> Vec<(String, Color)>  {
        let keywords = [
            "fn", "let", "mut", "if", "else", "match", "while", "loop", "for", "in", "return",
            "struct", "impl", "enum", "use", "mod", "pub", "crate", "const", "static", "as",
            "break", "continue", "trait", "where", "ref", "type",
        ];
        let types = ["usize", "String", "Result", "Option", "Vec", "i32", "u32", "bool"];
        
        let mut result = Vec::new();
        let mut i = 0;
        let chars: Vec<char> = line.chars().collect();
        while i< chars.len() {
            let c = chars[i];
            //Single line comment
            if c == '/' && i+1 < chars.len() && chars[i+1] == '/' {
                let comment: String = line[i..].to_string();
                result.push((comment, Color::DarkGrey));
                break;
            }
            //String literal
            if c == '"' {
                let start = i;
                i+=1;
                while i< chars.len() && chars[i] != '"' {
                    i+=1;
                }
                if i < chars.len(){
                    i+=1;
                }
                let quoted: String = chars[start..i].iter().collect();
                result.push((quoted, Color::Green));
                continue;
            }
            //Number
            if c.is_ascii_digit() {
                let start = i;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
                let number: String = chars[start..i].iter().collect();
                result.push((number, Color::Magenta));
                continue;
            }

            // Word (identifier/keyword/type)
            if c.is_alphanumeric() || c == '_' {
                let start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();
                let color = if keywords.contains(&word.as_str()) {
                    Color::Blue
                } else if types.contains(&word.as_str()) {
                    Color::Cyan
                } else {
                    Color::Reset
                };
                result.push((word, color));
                continue;
            }

            // Any other single char
            result.push((c.to_string(), Color::Reset));
            i += 1;
        }

        result
    }
    //save state
    fn snapshot(&self) -> EditorState {
        EditorState {
            buffer: self.rows.clone(),
            cursor_x: self.cursor_x,
            cursor_y: self.cursor_y,
        }
    }
    //saves rows from buffer
    fn restore(&mut self, state: EditorState) {
        self.rows = state.buffer;
        self.cursor_x = state.cursor_x;
        self.cursor_y = state.cursor_y;
    }
    fn push_undo(&mut self) {
        self.undo_stack.push(self.snapshot());
        self.redo_stack.clear(); // Clear redo history on new edit
    }
    //start search prompt
    fn start_search(&mut self) {
        self.search_mode = true;
        self.search_query.clear();
        self.search_results.clear();
        self.current_match = 0;
    }
    //search rows for query and keep it in search_results
    fn perform_search(&mut self){
        self.search_results.clear();
        if self.search_query.is_empty() {
            return;
        }
        let q = self.search_query.to_lowercase();
        for (i, line) in self.rows.iter().enumerate(){
            let line_lower = line.to_lowercase();
            let mut start = 0;
            while let Some(pos) = line_lower[start..].find(&q){
                self.search_results.push((i,start+pos)); //push into search_results if found
                start += pos+1; // continue searching
            }
        }
        self.current_match = 0;
        if let Some(&(row,col)) = self.search_results.get(0){
            self.cursor_x = row;
            self.cursor_y = col;
            self.scroll_to_cursor();
        }
    }

    fn scroll_to_cursor(&mut self) {
        let screen_cols = self.screen_cols as usize;
        if self.cursor_x < self.col_offset {
            self.col_offset = self.cursor_x;
        } else if self.cursor_x >= self.col_offset + screen_cols {
            self.col_offset = self.cursor_x - screen_cols + 1;
        }
    }
    fn draw_search_prompt(&self, stdout: &mut io::Stdout) -> std::io::Result<()> {
        use crossterm::style::{SetAttribute, Attribute, SetBackgroundColor, SetForegroundColor, Color};
        execute!(
            stdout,
            cursor::MoveTo(0, self.screen_rows - 1),
            Clear(ClearType::CurrentLine),
            SetBackgroundColor(Color::Black),
            SetForegroundColor(Color::Yellow),
            SetAttribute(Attribute::Bold),
            Print(format!("Search: {}", self.search_query)),
            SetAttribute(Attribute::Reset),
            SetForegroundColor(Color::Reset),
            SetBackgroundColor(Color::Reset),
        )
    }
    fn process_search_keypress(&mut self, event: KeyEvent) -> bool {
        if event.kind != KeyEventKind::Press {
            return false;
        }
        match event.code {
            KeyCode::Esc => {
                self.search_mode = false;
                self.search_query.clear();
                self.search_results.clear();
                return false;
            }
            KeyCode::Enter => {
                if self.search_results.is_empty() {
                    return false;
                }
                // Go to next match
                self.current_match = (self.current_match + 1) % self.search_results.len();
                let (row, col) = self.search_results[self.current_match];
                self.cursor_y = row;
                self.cursor_x = col;
                self.scroll_to_cursor();
            }
            KeyCode::Backspace => {
                self.search_query.pop();
                self.perform_search();
            }
            KeyCode::Char(c) => {
                self.search_query.push(c);
                self.perform_search();
            }
            _ => {}
        }
        false
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
    //Get filename
    if let Some(file) = env::args().nth(1){
        if Path::new(&file).exists() {
            editor.open(&file)?;
        } else {
            editor.filename = Some(file);
        }
    }
    // Main input loop
    loop {
        editor.refresh_screen(&mut stdout)?; // Redraw screen
        // Handling inputs
        if let Event::Key(key_event) = event::read()? {
            if editor.search_mode {
                editor.process_search_keypress(key_event);
            } else {
                if key_event.code == KeyCode::Char('f') && key_event.modifiers.contains(KeyModifiers::ALT){
                    editor.start_search();
                } else if editor.process_keypress(key_event){
                    break;
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
