# Scope2000

Scope2000 is the host application for the Viewer2000 rapid-control-prototyping
firmware. It provides parameter calibration, system commands, live monitoring,
triggered snapshots, waveform visualization, and CSV export.

## Project Scope

Scope2000 is the PC-side instrument for Viewer2000. It is not a firmware
flasher, device-specific motor-control wizard, or compatibility shell around
older protocols. Its first job is to exercise the Viewer2000 shared-interface
model through a real transport:

- enumerate firmware-published descriptors at runtime;
- stage and commit parameter transactions atomically;
- send system commands and report command results;
- stream native `ScopeBlock` data for Live monitoring;
- drain triggered Snapshot captures with pre-trigger history;
- render waveforms, mark stream gaps, and export CSV.

The native Viewer2000 protocol is the design authority. Compatibility with
other firmware is provided by an out-of-process bridge that exposes the same
service model through a generic byte-stream transport. Compatibility code does
not participate in the native SCI or future EtherCAT hot paths.

## Repository Relationship

Viewer2000 and Scope2000 are intentionally separate repositories under the same
workspace parent:

```text
20260610_Viewer2000/
  Viewer2000/   firmware, wire spec, shared contracts, golden vectors
  Scope2000/    Rust/egui host application and protocol conformance tests
```

The firmware repository owns the protocol definition:

- `Viewer2000/docs/wire-spec.md`
- `Viewer2000/contracts/`
- `Viewer2000/contracts/vectors/`

Scope2000 mirrors the golden vectors under `tests/vectors/` and treats them as
conformance tests. When Viewer2000 changes the wire specification, update the
Viewer2000 spec and vectors first, then run `tools/sync-vectors.sh` here and
update the Rust codec.

## UI Architecture

Scope2000 provides dockable variable, controller, acquisition, blueprint,
selection, console, and waveform panels with persistent workspace layout. Its
runtime catalog, parameter traffic, system commands, bindings, and scope blocks
come exclusively from the native Viewer2000 service model through `V2kSource`.

Legacy hardware protocols, connection clients, and firmware file workflows do
not belong in this process. They must be translated by a separate bridge and
must not change Viewer2000 protocol semantics, native sample widths, polling
cadence, SCI behavior, future EtherCAT assumptions, or UI data-path
performance.

## Development

```bash
cargo test
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo run
```

Use `cargo run` for local GUI bring-up. The initial transport is SCI over the
LAUNCHXL-F28P65X XDS110 virtual COM port; later transports must preserve the
same service semantics.

`V2kSource` is split into service semantics, message codec, and byte transport.
The initial transport is SCI. `ScopeBlock` keeps native sample widths and block
metadata until the plot or CSV boundary. A generic local byte-stream endpoint
is reserved for a future out-of-process compatibility bridge.

## License

Licensed under either Apache License 2.0 or MIT, at your option. The application
icon is original artwork owned by the project author and distributed under the
same dual license.
