# follow-demo

Two-arm scripted held-follow. Same-instant first burst is
registration-set order (`chanvoy_wait` before `sms_inbound` even when
the script lists SMS first). A later SMS event is a second burst.
The request deadline ends the session.

```
waitprims follow --registration-set registration_set.json --request live_wait_request.json --script follow.json
```

`make demo-follow` builds the diagnostic CLI and compares stdout to
`golden.jsonl`.
