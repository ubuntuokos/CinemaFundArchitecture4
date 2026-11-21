# Adapter process protocol

CFA4 runs device code that brings its own runtime or C library in a **child process**, so a crash
or a hang in that code ends the child, never the editor. Every such child speaks the same
line protocol with its supervisor (`cfa4-cli serve`). Today the control surface adapters use it
(`cfa4-surface-adapters`, plan CFA4-HW-CONTROL-SURFACES-001); later media input/output hosts
(capture and playout cards) follow the same rules with their own events and commands
(CFA4-DECKLINK-READY-001 V4.4).

Reference implementation: [`crates/surface-adapters/src/wire.rs`](../crates/surface-adapters/src/wire.rs).

## Transport

- One JSON object per line, UTF-8, `\n` terminated. The child writes **events** on stdout and
  reads **commands** on stdin. stderr is free text for logs; the supervisor never parses it.
- Every object has a `type` (camelCase). Field names are camelCase.
- End of stdin means `quit`: the child closes its devices and exits.
- A line the child cannot read is answered with an `error` event, code `bad_request`; the child
  keeps running.

## Handshake

The child's first line is `hello`:

```json
{"type":"hello","protocol":"cfa4-surface-adapter","protocolVersion":2,"family":"mirabox-family","kind":"surface","adapterVersion":"0.1.0","models":["ajazz.akp153e"]}
```

| Field            | Meaning                                                                  |
|------------------|--------------------------------------------------------------------------|
| `protocol`       | The protocol name (`cfa4-surface-adapter`)                               |
| `protocolVersion`| The version the child speaks (now 2)                                     |
| `family`         | The adapter family (`mirabox-family`); one process per family            |
| `kind`           | `surface` (control surfaces) or `io` (media input/output hosts)          |
| `adapterVersion` | The child's own version, for logs and bug reports                        |
| `models`         | The device models the family knows                                       |

The supervisor checks the hello before it reads anything else (`wire::handshake`). It reads the
line as plain JSON first, so it recognises a hello of any version:

- another `protocol`, another `protocolVersion` (version 1 children sent `version` instead), or
  another `kind` than the supervisor started: **`protocol_mismatch`**. The supervisor sends
  `quit`, stops the child, and reports the error with the family name. The editor keeps running.
- a first line that is not a hello: **`bad_request`**, handled the same way.

A version change is how the protocol grows: a new field that an older side could misread raises
`protocolVersion`, and both sides refuse what they do not speak, with a message, never a panic.
A purely optional new field (one an older reader can ignore) does not.

## Errors

`error` events carry a machine code and a message, optionally a `device` and the `seq` of the
command they answer: `permission_denied`, `device_busy`, `firmware_unknown`, `protocol_mismatch`,
`io_timeout`, `io_error`, `bad_request`, `device_not_found`, `unsupported`, `internal`. A panic
inside the child's device code becomes `internal` for that device; the process goes on.

## Commands with `seq`

A command may carry a `seq`; the child answers it with `ack` (same `seq`) or with an `error`
carrying that `seq`. Commands without `seq` are not answered unless they fail.

## Family-specific events and commands

The surface family (`kind: surface`) adds `connected`, `disconnected`, `input` events and
`setImage`, `clearImage`, `setBrightness`, `sleep` commands; see
[`crates/surface-adapters/README.md`](../crates/surface-adapters/README.md). An `io` host defines
its own on top of the same transport, handshake, errors and `seq` rules.
