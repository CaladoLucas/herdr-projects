# Delegated agents' visual origin

When a command inside a Herdr pane first delegates a task, Projects can record that pane as the visual creator of
its worker. The relationship uses one Herdr token, `spawn_origin`, written with source `plugin:herdr-projects`.
Its value is a JSON **string** containing this versioned tuple:

```json
[1,"expected-child-terminal","captured-parent-terminal"]
```

A consumer accepts only an array of exactly three elements: numeric version `1`, a nonempty child terminal string,
and a nonempty parent terminal string. It matches the second element to the observed child's immutable terminal
and resolves the third on the same current connection/runtime. Parent pane ID stays only in `creationOrigin`;
it is not needed in the token. Invalid, mismatched, stale or conflicting evidence is not a relation.
Shared relations, Board/team links and Town must all apply this binding. Projects never writes legacy `spawned_by`
or `spawned_by_terminal` alongside the new token: older World versions safely ignore this evidence.
World can consume these tokens without Projects contacting World or reading its authentication state.

This is display metadata. It does not send results, change permissions, select agents/models, route messages,
change task dependencies, or affect notifications. Existing team links and colors can also use the tokens.

## What counts as an origin

Both `HERDR_PANE_ID` and an explicit `HERDR_SOCKET_PATH` must be present at the first delegation. Projects queries
that exact local Unix socket, verifies the pane's immutable terminal ID, and verifies process ancestry: the pane's
shell belongs to the socket's kernel-identified server process, and the delegating command belongs to that shell.
Names, workspace membership and autopilot ownership never identify a creator. Forwarded/remote sockets, unsupported
peer-identity platforms, missing panes and unprovable process ancestry produce no visual origin.
Local peer identity is supported on macOS and Linux.

The task's `creationOrigin` stores the socket, canonical endpoint identity (device, inode and change timestamp),
server PID, pane ID and terminal ID before background launch or waiting for a slot/dependency. `false` records an
unverified or absent origin. That decision persists across retries and new rounds; it is never replaced with a retry
caller's identity. Attempts copy the same origin. Older tasks that already started or queued do not adopt a creator
retroactively. The child's inherited environment is never read to discover its parent.

## Writing safely

Before starting/restarting the worker, Projects verifies the saved endpoint/server identity and both exact terminal
IDs. A replaced socket, restarted runtime, removed parent, reused pane, missing terminal or self-reference skips the
write. Each request uses a new connection because Herdr closes a connection after one response; endpoint identity is
checked before and after every request, and the kernel peer PID must match before sending each request.

The tuple is serialized before writing. If its UTF-8 encoding exceeds 80 bytes, Projects skips the write without
affecting execution. It never truncates evidence; this conservative bound fits Herdr 0.9.5's token limit.

Only `spawn_origin` is added. Any existing `spawn_origin`, `spawned_by` or `spawned_by_terminal` key observed on the
child prevents writing, including malformed/empty values and claims from other sources. Summary, progress and
unrelated tokens are not cleared. Sidebar updates remain additive. A metadata error never fails the task; a later
worker restart can try the same saved origin again. A conflicting claim detected after writing returns a best-effort
failure without clearing, restoring or retrying tokens.

Herdr's metadata API does not provide a conditional write by terminal ID. Identity checks and the write are separate
requests. If the child changes from A to B exactly during the write, the token can physically land on B but still
names A in its second element: every conforming consumer must reject it for B. The post-write check detects the
replacement; it is not the safety mechanism and never tries to undo another pane's metadata.

There is also no atomic compare-and-set for claims. Observed claims are preserved; legacy keys are never touched,
including a legacy claim appearing during the write. Concurrent writers of the same `spawn_origin` key can still
race (last writer wins). A conflict visible afterward is reported as a no-op result without destructive repair.
Child binding prevents pane replacement from making either writer's evidence apply to a different child.

## Isolated verification

Run `npm run check delegate-origin` through the normal check harness with a scratch `HERDR_PROJECTS_HOME` and
`HERDR_BIN_PATH=/bin/false`. The tests use synthetic panes and fake agents; a small Unix server additionally closes
its connection after every reply to exercise the actual transport and kernel peer identity. No installed board or
Herdr session is used. The suite covers capture, missing/false origin, wrong socket/server, parent/child reuse,
queue, retries, attempts, token ownership, additive sidebar updates, nonfatal failure, replacement exactly during
writing, concurrent-claim preservation/conflict and the 80-byte serialized limit (including UTF-8 and escaping). The race tests are deterministic callbacks, not timed threads.

For a separate real integration, use a disposable Herdr configuration/session and board. Sanitize inherited
`HERDR_*` variables, set `XDG_CONFIG_HOME`, `HERDR_SOCKET_PATH` and `HERDR_PROJECTS_HOME` explicitly to disposable
locations, and prove the effective server/socket before any write. `HERDR_CONFIG_PATH` alone does not isolate the
socket. Leave `HOME` unchanged. Invoke the candidate inside a synthetic pane so its process ancestry can be verified;
invoking it from an unrelated shell deliberately yields no origin. Parse `spawn_origin` as JSON and compare both
terminal identities against the expected child and captured creator; assert that neither legacy token was emitted.
Test a replacement child with the original token: the consumer must reject it even if the parent still exists.
Also test parent removal/replacement, conflicting evidence and retry. Do not install the candidate over active
plugins or use existing user panes to demonstrate the relationship.
