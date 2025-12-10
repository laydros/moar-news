# Moar News

A lightweight, self-hosted RSS feed aggregator that displays multiple feeds in a clean, columnar dashboard. Built with Rust for performance and reliability.

Inspired by [spike.news](https://spike.news), which I liked but had issues with links getting mixed up between feeds. This is my take on the same concept.

## Features

- **Multi-feed dashboard** - View all your feeds side-by-side in a responsive grid
- **Automatic refresh** - Background task fetches new items at configurable intervals
- **Discussion links** - Special handling for Hacker News and Lobste.rs to show discussion links
- **Light/Dark themes** - Automatic theme switching based on system preference
- **HTMX-powered** - Smooth, partial page updates without full reloads
- **SQLite storage** - Self-contained database with no external dependencies
- **Low resource usage** - Runs happily on minimal hardware

## Quick Start

### Prerequisites

- Rust 1.70+ (install via [rustup](https://rustup.rs/))

### Running Locally

1. Clone the repository:
   ```bash
   git clone https://github.com/laydros/moar-news.git
   cd moar-news
   ```

2. Configure your feeds in `feeds.toml`:
   ```toml
   refresh_interval = 15  # minutes

   [[feeds]]
   name = "Hacker News"
   url = "https://news.ycombinator.com/rss"
   has_discussion = true

   [[feeds]]
   name = "Your Favorite Blog"
   url = "https://example.com/feed.xml"
   ```

3. Build and run:
   ```bash
   cargo run --release
   ```

4. Open http://localhost:3000 in your browser

## Configuration

### feeds.toml

| Field | Description |
|-------|-------------|
| `refresh_interval` | How often to fetch feeds (in minutes) |
| `[[feeds]]` | Feed definition block |
| `name` | Display name for the feed |
| `url` | RSS/Atom feed URL |
| `has_discussion` | Optional. Set to `true` for aggregators like HN/Lobste.rs to show discussion links |

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `DATABASE_URL` | `sqlite:moar_news.db?mode=rwc` | SQLite database connection string |
| `RUST_LOG` | `moar_news=info,tower_http=debug` | Logging level configuration |

## Building

### Development Build
```bash
cargo build
```

### Release Build
```bash
cargo build --release
```

The binary will be at `target/release/moar-news`.

### Running Tests
```bash
cargo test
```

## Deployment

### Deploy to Fly.io

Fly.io is the recommended hosting platform for Moar News. It offers a generous free tier and handles persistent storage for SQLite.

#### First-Time Setup

1. Install the Fly CLI:
   ```bash
   # macOS
   brew install flyctl

   # Linux
   curl -L https://fly.io/install.sh | sh

   # Windows
   powershell -Command "iwr https://fly.io/install.ps1 -useb | iex"
   ```

2. Sign up / Log in:
   ```bash
   fly auth signup
   # or
   fly auth login
   ```

3. Launch the app (first time only):
   ```bash
   fly launch --no-deploy
   ```
   - When prompted, choose a unique app name (or accept the generated one)
   - Select a region close to you
   - Say **No** to adding databases (we use SQLite)
   - Say **No** to deploying now

4. Create the persistent volume for SQLite:
   ```bash
   fly volumes create moar_news_data --region <your-region> --size 1
   ```

5. Deploy:
   ```bash
   fly deploy
   ```

6. Open your app:
   ```bash
   fly open
   ```

#### Subsequent Deployments

After the initial setup, just run:
```bash
fly deploy
```

Or push to `main` to trigger automatic deployment (see below).

### Automatic Deploys via GitHub Actions

The repository includes a GitHub Actions workflow that automatically deploys to Fly.io when you push to `main`.

#### Setup

1. Get your Fly.io API token:
   ```bash
   fly tokens create deploy -x 999999h
   ```

2. Add the token to your GitHub repository:
   - Go to Settings > Secrets and variables > Actions
   - Click "New repository secret"
   - Name: `FLY_API_TOKEN`
   - Value: (paste your token)

3. Push to `main`:
   ```bash
   git push origin main
   ```

The workflow will automatically build and deploy your app.

### Deploy with Docker

You can also run Moar News anywhere Docker is supported:

```bash
# Build the image
docker build -t moar-news .

# Run with a persistent volume
docker run -d \
  -p 3000:3000 \
  -v moar-news-data:/data \
  -e DATABASE_URL="sqlite:/data/moar_news.db?mode=rwc" \
  moar-news
```

### Deploy to a VPS

1. Build the release binary locally or on the server:
   ```bash
   cargo build --release
   ```

2. Copy files to your server:
   ```bash
   scp target/release/moar-news user@server:/opt/moar-news/
   scp feeds.toml user@server:/opt/moar-news/
   scp -r static user@server:/opt/moar-news/
   ```

3. Create a systemd service (`/etc/systemd/system/moar-news.service`):
   ```ini
   [Unit]
   Description=Moar News RSS Aggregator
   After=network.target

   [Service]
   Type=simple
   User=www-data
   WorkingDirectory=/opt/moar-news
   ExecStart=/opt/moar-news/moar-news
   Restart=on-failure
   Environment=RUST_LOG=moar_news=info

   [Install]
   WantedBy=multi-user.target
   ```

4. Start the service:
   ```bash
   sudo systemctl enable moar-news
   sudo systemctl start moar-news
   ```

5. Set up a reverse proxy (nginx/caddy) for HTTPS.

## Usage

### Web Interface

- **Main view** (`/`) - Dashboard showing all feeds in columns
- **Load more** - Click "Load more" at the bottom of any feed column
- **Refresh** - Click the refresh button to manually fetch all feeds
- **Theme** - Automatically matches your system light/dark preference

### API Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/` | GET | Main dashboard |
| `/feed/:id/more?offset=N` | GET | Load more items for a feed (HTMX) |
| `/refresh` | POST | Trigger manual feed refresh |
| `/refresh/status` | GET | Check if refresh is in progress |
| `/health` | GET | Health check endpoint |

## Project Structure

```
moar-news/
├── src/
│   ├── main.rs       # Application entry point
│   ├── config.rs     # Configuration loading
│   ├── db.rs         # Database operations
│   ├── fetcher.rs    # Feed fetching logic
│   └── routes.rs     # HTTP route handlers
├── templates/        # Askama HTML templates
├── static/           # CSS and favicon
├── feeds.toml        # Feed configuration
├── Dockerfile        # Container build
├── fly.toml          # Fly.io configuration
└── .github/workflows # CI/CD
```

## AI use
Project made with LLM assistance. I respect your choice not to use it if that
bothers you. But it works well for my brain.

## Support
This is a personal project; issues/PRs are welcome but not guarantees on response time.

## License

GPLv3
