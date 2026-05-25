# Google Play API Rust Library

A Rust library for interacting with the Google Play API.

This would not be possible without the excellent work of [AuroraOSS's GPlayApi](https://gitlab.com/AuroraOSS/gplayapi) and [EFF's rs-google-play](https://github.com/EFForg/rs-google-play), both of which this project draws from and was inspired by.

The protobuf files and device configs are sourced from both projects. The protobuf definitions have been modified for snake_case naming and a few Rust compatibility changes.

This is an unofficial client for interacting with Google Play APIs. It is not affiliated with, endorsed by, or supported by Google. Users are responsible for complying with Google Play’s terms and any applicable laws or third-party licenses.

## Notes

- I try to keep this reasonably up to date with relevant changes from Aurora and EFF.
- This project is primarily for my own use, so I cannot guarantee that breaking changes will not happen.
- I did not originally plan to open-source this, so the git history was wiped when publishing.
- Not all code paths have been tested, especially areas I do not personally use.
- I also have a python version I update in tandem: [pyplay](https://github.com/Andell4301/pyplay)

I'm still fairly new to Rust, so there may be some rough edges.