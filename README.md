# SpeedReader

A CLI tool for speed reading text with AI comprehension evaluation.

## Installation

### Using Cargo

```bash
cargo install speedreader
```

### Manual Installation

1. Clone the repository:

```bash
git clone https://github.com/fcasibu/speedreader.git
cd speedreader
```

2. Build the project:

```bash
cargo build --release
```

3. The binary will be available at `target/release/speedreader`

## Usage

### Basic Reading

Read a file at the default speed (258 WPM):

```bash
speedreader --file path/to/text.txt
```

Read at a specific WPM:

```bash
speedreader --file path/to/text.txt --wpm 300
```

Read from stdin:

```bash
cat path/to/text.txt | speedreader
# or through a clipboard (example is what I use)
xclip -selection clipboard -o | speereader
```

### Configuration

SpeedReader supports a configuration file for customizing various settings. To generate a default configuration file:

```bash
speedreader --init-config
```

This creates a configuration file at `~/.config/speedreader/config.toml` (Linux/macOS) or `%APPDATA%\speedreader\config.toml` (Windows).

The configuration file allows you to customize:

- Default words per minute (WPM)
- WPM adjustment step size
- Keybindings for pause, quit, and WPM adjustment
- AI model for summary evaluation

Example configuration:

```toml
# Words per minute
wpm = 258
# Step to adjust WPM when using + and - keys
wpm_step = 5
# AI model to use for summary evaluation
model = "deepseek/deepseek-r1:free"

# Keybindings configuration
[keys]
# Key to quit
quit = "q"
# Key to pause/resume
pause = " "
# Key to increase WPM
increase_wpm = "+"
# Key to decrease WPM
decrease_wpm = "-"
```

### Controls During Reading

Default keybindings (can be customized in the config file):

- **Space**: Pause/resume reading
- **Q**: Quit reading
- **+**: Increase WPM
- **-**: Decrease WPM

### AI Analysis

After finishing reading, you'll be prompted to enter a summary of what you read. The summary will be analyzed by an AI model and you'll receive feedback on your comprehension.

For this feature to work, you must have an OpenRouter API key set as an environment variable:

```bash
export OPEN_ROUTER_API_KEY=your_api_key_here
```

## Command-line Options

```
Options:
  -f, --file <FILE>  Path of the text file to speed read
      --wpm <WPM>    Words per minute
      --init-config  Generate a default config file
  -h, --help         Print help
```

## Requirements

- The AI summary evaluation feature requires an [OpenRouter API key](https://openrouter.ai/) set as the `OPEN_ROUTER_API_KEY` environment variable.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## License

This project is licensed under the MIT License. See the [LICENSE](./LICENSE) file for details.
