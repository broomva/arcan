# arcan-prosopon

`Pneuma<L0ToExternal>` for [Arcan](../arcan). Subscribes to the runtime's
`aios_protocol::EventRecord` broadcast, translates each `EventKind` into a
`prosopon_core::ProsoponEvent`, and publishes envelopes into a
`prosopon_daemon::EnvelopeFanout` for downstream compositors (text, glass,
field, …).

Full design: [`docs/superpowers/plans/2026-04-23-bro-773-arcan-prosopon.md`](../../../docs/superpowers/plans/2026-04-23-bro-773-arcan-prosopon.md).
Linear: [BRO-773](https://linear.app/broomva/issue/BRO-773).
