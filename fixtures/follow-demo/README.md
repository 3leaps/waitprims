# follow-demo

Two-arm scripted held-follow. Run every command from the **repository
root**. Same-instant first burst is registration-set order
(`chanvoy_wait` before `sms_inbound` even when `follow.json` lists SMS
first). A later SMS event is a second burst. The request
`run_deadline` is `2026-08-15T16:20:00Z`, after the last scripted event
at `2026-08-15T16:10:00Z`, so the session ends at that deadline.

`sequence` is added by the CLI. It counts accepted bursts (1, then 2).
Event bodies and `proposed_next_anchor` values come from `follow.json`.
The last stdout line is a runtime-only `follow_end` / `deadline` view.
It is not a seventh `agent-wait/v0` message.

Canonical check:

```bash
make demo-follow
```

That builds `./target/debug/waitprims` with
`cargo build --locked --offline -p waitprims-cli`, runs the command
below with `--log-level error`, and compares stdout to `golden.jsonl`.

From a repository-root debug binary:

```bash
./target/debug/waitprims --log-level error follow \
  --registration-set fixtures/follow-demo/registration_set.json \
  --request fixtures/follow-demo/live_wait_request.json \
  --script fixtures/follow-demo/follow.json
```

After `cargo install --path crates/waitprims-cli --locked --force`, the
same root-relative paths work with the installed name:

```bash
waitprims --log-level error follow \
  --registration-set fixtures/follow-demo/registration_set.json \
  --request fixtures/follow-demo/live_wait_request.json \
  --script fixtures/follow-demo/follow.json
```
