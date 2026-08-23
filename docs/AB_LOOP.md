<!--
SPDX-FileCopyrightText: 2026 Jamelade contributors
SPDX-License-Identifier: GPL-3.0-or-later
-->

# A–B loop

In the expanded player, the **A–B** button cycles through three states:

1. Set the start point (A).
2. Set the end point (B) and begin looping.
3. Clear both points.

A and B appear on the seek bar. The existing track and queue repeat modes stay
independent. A loop needs at least one second between its points and is cleared
when the track, queue, session, or playback sidecar changes.

The state is process-local and is never written to disk. While an active loop is
playing, a short native timer seeks back to A after playback reaches B; the timer
stops when the loop is cleared or playback pauses.

The state machine and timing safeguards are in `src/segment_loop.rs`; GTK and
playback integration are in `src/app/segment_loop.rs` and
`src/components/player_view/transport.rs`.
