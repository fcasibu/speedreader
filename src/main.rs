use clap::Parser;
use console::Term;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode},
    execute, terminal,
};
use dirs::config_dir;
use indicatif::{ProgressBar, ProgressStyle};
use serde::{Deserialize, Serialize};
use std::{
    cmp, env,
    fmt::Display,
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    thread, time,
};
use thiserror::Error;
use unicode_width::UnicodeWidthStr;

const MAX_WPM: u64 = 1000;
const MIN_WPM: u64 = 150;

#[derive(Debug, Error)]
enum SpeedReaderError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("Time error: {0}")]
    SystemTimeError(#[from] time::SystemTimeError),

    #[error("API error: {0}")]
    ReqwestError(#[from] reqwest::Error),

    #[error("Template error: {0}")]
    TemplateError(#[from] indicatif::style::TemplateError),

    #[error("Environment variable error: {0}")]
    EnvVarError(#[from] env::VarError),

    #[error("Integer conversion error")]
    IntegerConversionError,

    #[error("File not found: {0}")]
    FileNotFound(String),

    #[error("API response error: {0}")]
    ApiResponseError(String),

    #[error("Event reading error")]
    EventReadingError,

    #[error("Config error: {0}")]
    ConfigError(String),

    #[error("TOML serialization error: {0}")]
    TomlSerError(#[from] toml::ser::Error),

    #[error("TOML deserialization error: {0}")]
    TomlDeError(#[from] toml::de::Error),
}

type Result<T> = std::result::Result<T, SpeedReaderError>;

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Config {
    /// Words per minute
    #[serde(default = "default_wpm")]
    wpm: u64,

    /// Step to adjust WPM when incremeting or decrementing
    #[serde(default = "default_wpm_step")]
    wpm_step: u64,

    /// AI model to use for summary evaluation
    #[serde(default = "default_model")]
    model: String,

    /// Keybindings configuration
    #[serde(default)]
    keys: KeyBindings,
}

fn default_wpm() -> u64 {
    258
}
fn default_wpm_step() -> u64 {
    5
}
fn default_model() -> String {
    "deepseek/deepseek-r1:free".to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct KeyBindings {
    /// Key to quit
    #[serde(default = "default_quit_key")]
    quit: char,

    /// Key to pause/resume
    #[serde(default = "default_pause_key")]
    pause: char,

    /// Key to increase WPM
    #[serde(default = "default_increase_wpm_key")]
    increase_wpm: char,

    /// Key to decrease WPM
    #[serde(default = "default_decrease_wpm_key")]
    decrease_wpm: char,
}

fn default_quit_key() -> char {
    'q'
}
fn default_pause_key() -> char {
    ' '
}
fn default_increase_wpm_key() -> char {
    '+'
}
fn default_decrease_wpm_key() -> char {
    '-'
}

impl Default for KeyBindings {
    fn default() -> Self {
        KeyBindings {
            quit: default_quit_key(),
            pause: default_pause_key(),
            increase_wpm: default_increase_wpm_key(),
            decrease_wpm: default_decrease_wpm_key(),
        }
    }
}

impl Config {
    fn load() -> Result<Self> {
        let config_path = get_config_path()?;

        if !config_path.exists() {
            let config = Config::default();
            config.save()?;
            return Ok(config);
        }

        let config_str = fs::read_to_string(&config_path).map_err(|e| {
            SpeedReaderError::ConfigError(format!("Failed to read config file: {}", e))
        })?;

        let config: Config = toml::from_str(&config_str)?;

        Ok(config)
    }

    fn save(&self) -> Result<()> {
        let config_path = get_config_path()?;

        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                SpeedReaderError::ConfigError(format!("Failed to create config directory: {}", e))
            })?;
        }

        let config_str = toml::to_string_pretty(self)?;

        fs::write(&config_path, config_str).map_err(|e| {
            SpeedReaderError::ConfigError(format!("Failed to write config file: {}", e))
        })?;

        Ok(())
    }

    fn from_args(args: &Args) -> Result<Self> {
        let mut config = Self::load()?;

        if let Some(wpm) = args.wpm {
            config.wpm = cmp::min(MAX_WPM, cmp::max(wpm, MIN_WPM));
        }

        Ok(config)
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            wpm: default_wpm(),
            wpm_step: default_wpm_step(),
            model: default_model(),
            keys: KeyBindings::default(),
        }
    }
}

fn get_config_path() -> Result<PathBuf> {
    let mut path = config_dir().ok_or_else(|| {
        SpeedReaderError::ConfigError("Failed to find config directory".to_string())
    })?;

    path.push("speedreader");
    path.push("config.toml");

    Ok(path)
}

#[derive(Debug, Clone, Copy)]
enum TextAlignment {
    Left,
    Center,
    Right,
}

#[derive(Parser)]
struct Args {
    /// Path of the text file to speed read
    #[arg(short, long)]
    file: Option<String>,

    /// Words per minute
    #[arg(long)]
    wpm: Option<u64>,

    /// Generate a default config file
    #[arg(long)]
    init_config: bool,
}

#[derive(Serialize, Deserialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Serialize, Deserialize)]
struct OpenRouterBody {
    model: String,
    messages: Vec<Message>,
}

#[derive(Deserialize)]
struct ApiResponse {
    choices: Option<Vec<Choice>>,
}

#[derive(Deserialize)]
struct Choice {
    message: Option<Content>,
}

#[derive(Deserialize)]
struct Content {
    content: String,
}

struct ReadResult {
    success: bool,
    wpm: Option<u64>,
}

const OPEN_ROUTER_URL: &str = "https://openrouter.ai/api/v1/chat/completions";

fn print_text<T: Display>(
    text: T,
    position: (u16, u16),
    text_alignment: TextAlignment,
) -> Result<()> {
    let mut stdout = io::stdout();
    let (columns, rows) = position;
    let width = text
        .to_string()
        .width()
        .try_into()
        .map_err(|_| SpeedReaderError::IntegerConversionError)?;

    let col_pos = match text_alignment {
        TextAlignment::Left => columns,
        TextAlignment::Center => columns.saturating_sub(width / 2),
        TextAlignment::Right => columns.saturating_sub(width),
    };

    execute!(stdout, cursor::MoveTo(col_pos, rows))?;
    print!("{}", text);
    stdout.flush()?;

    Ok(())
}

fn tokenize_text(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|word| word.chars().filter(|c| c.is_alphanumeric()).collect())
        .collect()
}

fn display_countdown(size: (u16, u16), seconds: u64) -> Result<()> {
    let (columns, rows) = size;
    let countdown = time::Duration::from_secs(seconds);
    let start = time::SystemTime::now();

    loop {
        let elapsed = start.elapsed()?;
        let remaining = countdown.as_secs().saturating_sub(elapsed.as_secs());

        print_text(remaining, (columns / 2, rows / 2), TextAlignment::Center)?;

        if remaining > 0 {
            print_text(
                "Starting in...",
                (columns / 2, rows / 2 - 2),
                TextAlignment::Center,
            )?;
        }

        thread::sleep(time::Duration::new(1, 0));

        if elapsed >= countdown {
            break;
        }
    }

    Ok(())
}

fn handle_paused_input(
    current_wpm: &mut u64,
    paused: &mut bool,
    size: (u16, u16),
    config: &Config,
) -> Result<Option<ReadResult>> {
    let (columns, rows) = size;
    let mut stdout = io::stdout();

    print_text(
        format!(
            "Paused. Press \"{}\" to resume...",
            if config.keys.pause == ' ' {
                "Spacebar".to_string()
            } else {
                config.keys.pause.to_string()
            }
        ),
        (columns / 2, rows / 2 + 1),
        TextAlignment::Center,
    )?;

    while *paused {
        if event::poll(time::Duration::from_millis(100))? {
            match event::read() {
                Ok(Event::Key(key_code)) => match key_code.code {
                    KeyCode::Char(c) if c == config.keys.increase_wpm => {
                        *current_wpm = cmp::min(*current_wpm + config.wpm_step, MAX_WPM);
                        print_text(format!("WPM: {current_wpm}"), (0, 0), TextAlignment::Left)?;
                    }
                    KeyCode::Char(c) if c == config.keys.decrease_wpm => {
                        *current_wpm = cmp::max(*current_wpm - config.wpm_step, MIN_WPM);
                        execute!(stdout, cursor::MoveTo(0, 0))?;
                        print!("{}", " ".repeat(format!("WPM: {MAX_WPM}").len()));
                        print_text(format!("WPM: {current_wpm}"), (0, 0), TextAlignment::Left)?;
                    }
                    KeyCode::Char(c) if c == config.keys.quit => {
                        return Ok(Some(ReadResult {
                            success: false,
                            wpm: None,
                        }));
                    }
                    KeyCode::Char(c) if c == config.keys.pause => {
                        *paused = false;
                        execute!(stdout, terminal::Clear(terminal::ClearType::CurrentLine))?;
                    }
                    _ => continue,
                },
                Ok(_) => continue,
                Err(_) => return Err(SpeedReaderError::EventReadingError),
            };
        }
    }

    Ok(None)
}

fn display_word_ui(
    word: &str,
    current_word: usize,
    total_words: usize,
    current_wpm: u64,
    size: (u16, u16),
    config: &Config,
) -> Result<()> {
    let mut stdout = io::stdout();
    let (columns, rows) = size;

    execute!(stdout, terminal::Clear(terminal::ClearType::All))?;

    print_text(format!("WPM: {current_wpm}"), (0, 0), TextAlignment::Left)?;

    print_text(
        format!(
            "Word {current_word_index} / {total}",
            current_word_index = current_word + 1,
            total = total_words
        ),
        (columns, 0),
        TextAlignment::Right,
    )?;

    print_text(word, (columns / 2, rows / 2), TextAlignment::Center)?;

    print_text(
        format!(
            "Controls: {}=Pause, {}=Quit, {}/{} = Adjust WPM",
            if config.keys.pause == ' ' {
                "Spacebar".to_string()
            } else {
                config.keys.pause.to_string()
            },
            config.keys.quit,
            config.keys.increase_wpm,
            config.keys.decrease_wpm
        ),
        (columns / 2, rows - 2),
        TextAlignment::Center,
    )?;

    Ok(())
}

fn speed_read(buf: &String, config: &Config, size: (u16, u16)) -> Result<ReadResult> {
    let mut current_wpm = config.wpm;
    let (columns, rows) = size;

    display_countdown(size, 3)?;

    let mut paused = false;

    let words = tokenize_text(buf);

    for (i, word) in words.iter().enumerate() {
        display_word_ui(word, i, words.len(), current_wpm, size, config)?;

        let dur_wpm = time::Duration::from_millis(60_000 / current_wpm);
        let start = time::Instant::now();

        while start.elapsed().as_millis() < dur_wpm.as_millis() {
            if event::poll(time::Duration::from_millis(50))? {
                match event::read() {
                    Ok(Event::Key(key_code)) => match key_code.code {
                        KeyCode::Char(c) if c == config.keys.quit => {
                            return Ok(ReadResult {
                                success: false,
                                wpm: None,
                            });
                        }
                        KeyCode::Char(c) if c == config.keys.pause => {
                            paused = !paused;
                        }
                        _ => continue,
                    },
                    Ok(_) => continue,
                    Err(_) => return Err(SpeedReaderError::EventReadingError),
                };
            }

            if paused {
                if let Some(result) =
                    handle_paused_input(&mut current_wpm, &mut paused, size, config)?
                {
                    if result.success && result.wpm.is_some() {
                        return speed_read(buf, config, (columns, rows));
                    }
                    return Ok(result);
                }
            }
        }
    }

    Ok(ReadResult {
        success: true,
        wpm: Some(current_wpm),
    })
}

fn create_evaluation_prompt(summary: &str, text: &str, wpm: u64) -> String {
    format!(
        r#"
Original Text:
"""
{text}
"""

User Summary:
"""
{summary}
"""

WPM: {wpm}

Based on the Original Text, please evaluate the User Summary. Assess its comprehension based on:
1. Accuracy: Does the summary correctly represent the information in the original text?
2. Key Points Coverage: Does the summary include the main ideas and crucial supporting details?
3. Completeness: How much of the core information is captured?
4. Misinterpretations: Are there any points that are clearly misunderstood?

Provide:
- A qualitative rating (e.g., Excellent, Good, Fair, Poor).
- A list of key points correctly captured in the summary.
- A list of significant points from the original text that were missed or misrepresented in the summary.
- A brief overall comment on the user's comprehension based on their WPM.
        "#
    )
}

fn get_api_key() -> Result<String> {
    let api_key =
        env::var("OPEN_ROUTER_API_KEY").map_err(|err| SpeedReaderError::EnvVarError(err))?;

    if api_key.trim().is_empty() {
        eprintln!("Error: OPEN_ROUTER_API_KEY environment variable is set but empty");
        return Err(SpeedReaderError::EnvVarError(env::VarError::NotPresent));
    }

    Ok(api_key)
}

async fn send_evaluation_request(
    client: &reqwest::Client,
    message: Message,
    api_key: &str,
    model: &str,
    progress_bar: &ProgressBar,
) -> Result<String> {
    let response = client
        .post(OPEN_ROUTER_URL)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .json(&OpenRouterBody {
            model: model.to_string(),
            messages: vec![message],
        })
        .send()
        .await?;

    if response.status().is_success() {
        progress_bar.set_message("Parsing AI response...");
        let data: ApiResponse = response.json().await?;

        let ai_response = match &data.choices {
            Some(choices) if !choices.is_empty() => match &choices[0].message {
                Some(message) => message.content.clone(),
                None => {
                    return Err(SpeedReaderError::ApiResponseError(
                        "Missing message content in API response".to_string(),
                    ));
                }
            },
            Some(_) => {
                return Err(SpeedReaderError::ApiResponseError(
                    "Empty choices array in API response".to_string(),
                ));
            }
            None => {
                return Err(SpeedReaderError::ApiResponseError(
                    "Missing choices in API response".to_string(),
                ));
            }
        };

        Ok(ai_response)
    } else {
        let error_text = response.text().await?;
        Err(SpeedReaderError::ApiResponseError(format!(
            "API request failed: {error_text}"
        )))
    }
}

#[tokio::main]
async fn process_summary(summary: String, text: String, wpm: u64, config: &Config) -> Result<()> {
    let pb = ProgressBar::new_spinner();
    pb.enable_steady_tick(time::Duration::from_millis(120));
    pb.set_style(ProgressStyle::default_spinner().template("{spinner:.blue} {msg}")?);
    pb.set_message("\nSetting up evaluation...");

    let api_key = get_api_key()?;

    let client = reqwest::Client::new();

    let prompt = create_evaluation_prompt(&summary, &text, wpm);
    let message = Message {
        role: "user".to_string(),
        content: prompt,
    };

    pb.set_message("Sending request to AI for evaluation...");

    match send_evaluation_request(&client, message, &api_key, &config.model, &pb).await {
        Ok(ai_response) => {
            pb.finish_with_message("AI analysis complete!");
            println!("{ai_response}");
            Ok(())
        }
        Err(e) => {
            pb.finish_with_message("API request failed!");
            println!("Error: {e}");
            Err(e)
        }
    }
}

fn main() -> Result<()> {
    let args = Args::parse();

    if args.init_config {
        let config = Config::default();
        config.save()?;
        println!(
            "Default configuration created at: {}",
            get_config_path()?.display()
        );
        return Ok(());
    }

    let config = Config::from_args(&args)?;

    let mut text_buf = String::new();

    if let Some(file_path) = args.file.as_ref() {
        if !Path::new(file_path).exists() {
            eprintln!("Error: The file '{}' does not exist.", file_path);
            return Err(SpeedReaderError::FileNotFound(file_path.to_string()));
        }

        text_buf = fs::read_to_string(file_path)?;
    } else {
        let stdin_result = io::stdin().read_to_string(&mut text_buf);
        if let Err(e) = stdin_result {
            eprintln!("Error reading from stdin: {}", e);
            return Err(SpeedReaderError::Io(e));
        }
    }

    if text_buf.trim().is_empty() {
        eprintln!(
            "Error: No text provided. Please provide a file with text or pipe text to stdin."
        );
        return Err(SpeedReaderError::ApiResponseError(
            "No text provided".to_string(),
        ));
    }

    let size = terminal::size()?;
    let mut stdout = io::stdout();

    execute!(stdout, terminal::EnterAlternateScreen)?;
    execute!(stdout, terminal::Clear(terminal::ClearType::All))?;
    execute!(stdout, cursor::Hide)?;
    terminal::enable_raw_mode()?;

    let run_result = (|| {
        let result = speed_read(&text_buf, &config, size)?;
        Ok(result)
    })();

    let terminal_reset_result = || -> Result<()> {
        terminal::disable_raw_mode()?;
        execute!(stdout, terminal::LeaveAlternateScreen)?;
        execute!(stdout, terminal::Clear(terminal::ClearType::All))?;
        execute!(stdout, cursor::MoveTo(0, 0))?;
        execute!(stdout, cursor::Show)?;
        Ok(())
    }();

    if let Err(e) = terminal_reset_result {
        eprintln!("Error restoring terminal state: {}", e);
        return Err(e);
    }

    match run_result {
        Ok(result) => {
            if result.success {
                if let Some(wpm) = result.wpm {
                    println!("Please enter your summary of the text. Press Enter to finish.");
                    println!("Enter your summary below:");

                    let term = Term::stdout();
                    let mut summary_buf = String::new();

                    loop {
                        let line = term.read_line()?;
                        let line = line.trim_end();

                        if line.is_empty() {
                            break;
                        }

                        summary_buf.push_str(line);
                        summary_buf.push('\n');
                    }

                    if summary_buf.trim().is_empty() {
                        println!("No summary provided. Exiting.");
                        return Ok(());
                    }

                    if text_buf.trim().is_empty() {
                        println!("No summary provided. Exiting.");
                        return Ok(());
                    }

                    process_summary(summary_buf, text_buf, wpm, &config)?;
                }
            }
            Ok(())
        }
        Err(e) => {
            eprintln!("Error during speed reading: {}", e);
            Err(e)
        }
    }
}
