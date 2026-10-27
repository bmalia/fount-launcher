# Fount
Fount is a work-in-progress lightweight launcher backend for Linux systems. It's designed to be fast and efficient, while also being portable enough to wire into any frontend you need.

## Features
- Automatic indexing and merging of application entries
- Communication with frontends over Unix sockets
- Custom, JSON-defined launcher entry support

## Project Structure
The project is structured into 2 crates:
- `fount-core`: The main library crate that handles the core functionality of the launcher.
- `fount-daemon`: The application crate that handles threading and forntend communication. \
The daemon crate depends on the core crate, so if you're planning on using the launcher alone, you only need to pull the daemon crate.
