# auki-proto src

Current implementation: generated prost modules are checked in under
`src/generated/` and included from `src/lib.rs`. The crate also implements
`auki_logs::LogPayload` for log payload messages so existing log code can use
the generated records directly.
