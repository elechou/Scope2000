# Scope2000 Development Guidelines

- Scope2000 is the PC-side application for Viewer2000.
- The Viewer2000 wire specification and golden vectors are authoritative.
- Scope2000 is a native Viewer2000 UI first. Its runtime behavior and public
  terminology must follow the Viewer2000 service model.
- Keep native block samples in their original wire width until the display or
  export boundary.
- Legacy hardware support belongs in an out-of-process protocol bridge.
  Compatibility behavior must not enter `V2kSource`, alter Viewer2000 service
  semantics, or constrain the native protocol, data model, polling cadence,
  SCI path, future EtherCAT path, or performance.
- Public source, paths, UI strings, documentation, and commit messages use
  Scope2000 and Viewer2000 terminology.
- Use English for identifiers and commit messages. Comments may be Chinese
  during development.
