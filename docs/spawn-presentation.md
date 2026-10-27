# Delegated agents' visual origin

When a command inside a Herdr pane first delegates a task, Projects can record that pane as the visual creator of
its worker. The relationship uses the existing Herdr tokens `spawned_by` (creator pane ID) and
`spawned_by_terminal` (creator immutable terminal ID), written with source `plugin:herdr-projects`.
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

Only the two spawn tokens are added. Existing spawn claims are left intact, including claims written by other
sources; summary, progress and unrelated tokens are not cleared. Sidebar updates remain additive and preserve spawn
tokens. A metadata error never fails the task; a later worker restart can try the same saved origin again.

Herdr's metadata API does not provide a conditional write by terminal ID. Identity checks and the write are separate
requests, so concurrent pane replacement in the final read/write interval cannot be made transactional by Projects.
The immutable parent token remains essential for the consumer to reject stale evidence. Projects never clears or
reassigns an existing claim to try to recover from uncertainty.

## Isolated verification

Run `npm run check delegate-origin` through the normal check harness with a scratch `HERDR_PROJECTS_HOME` and
`HERDR_BIN_PATH=/bin/false`. The tests use synthetic panes and fake agents; a small Unix server additionally closes
its connection after every reply to exercise the actual transport and kernel peer identity. No installed board or
Herdr session is used. The suite covers capture, missing/false origin, wrong socket/server, parent/child reuse,
queue, retries, attempts, token ownership, additive sidebar updates and nonfatal failure.

For a separate real integration, use a disposable Herdr configuration/session and board. Sanitize inherited
`HERDR_*` variables, set `XDG_CONFIG_HOME`, `HERDR_SOCKET_PATH` and `HERDR_PROJECTS_HOME` explicitly to disposable
locations, and prove the effective server/socket before any write. `HERDR_CONFIG_PATH` alone does not isolate the
socket. Leave `HOME` unchanged. Invoke the candidate inside a synthetic pane so its process ancestry can be verified;
invoking it from an unrelated shell deliberately yields no origin. Check both token values against the creator's
current `pane.get` identity, then test parent removal/replacement and retry. Do not install the candidate over active
plugins or use existing user panes to demonstrate the relationship.
