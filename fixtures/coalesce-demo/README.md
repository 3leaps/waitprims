# coalesce-demo

Two-registration scripted held-coalesce. Run every command from the
**repository root**. `reg:sms-1` omits `priority` (quiet), and
`reg:urgent-1` declares `priority: 100` (urgent). The default policy
(`min_emit_interval = 10s`, `urgent_at = 100`) is used.

The script buffered a quiet event at `2026-08-15T16:05:00Z`, then an
urgent event arrived at `2026-08-15T16:15:00Z`. The urgent emit does not
wait for the quiet window, so it flushes immediately as burst 1 while the
quiet event is still buffered; the quiet buffer flushes later as burst 2.
The request `run_deadline` is `2026-08-15T16:20:00Z`, after the last
scripted event, so the session ends at that deadline.

`sequence` is added by the CLI. It counts accepted bursts (1, then 2).
Event bodies and `proposed_next_anchor` values come from `coalesce.json`.
The last stdout line is a runtime-only `follow_end` / `deadline` view.
It is not a seventh `agent-wait/v0` message.

Canonical check:

```bash
make demo-coalesce
```

That builds `./target/debug/waitprims` with
`cargo build --locked --offline -p waitprims-cli`, runs the command
below with `--log-level error`, and compares stdout to `golden.jsonl`.

From a repository-root debug binary:

```bash
./target/debug/waitprims --log-level error coalesce \
  --registration-set fixtures/coalesce-demo/registration_set.json \
  --request fixtures/coalesce-demo/live_wait_request.json \
  --script fixtures/coalesce-demo/coalesce.json
```

After `cargo install --path crates/waitprims-cli --locked --force`, the
same root-relative paths work with the installed name:

```bash
waitprims --log-level error coalesce \
  --registration-set fixtures/coalesce-demo/registration_set.json \
  --request fixtures/coalesce-demo/live_wait_request.json \
  --script fixtures/coalesce-demo/coalesce.json
```
