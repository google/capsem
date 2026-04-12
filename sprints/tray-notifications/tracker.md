# Tray Notifications Sprint (S13) -- Attention & Alerts

Add notification support to capsem-tray: macOS native notifications for events, a badge dot on the tray icon when user input is needed, and a subtle pulse animation to draw attention without being intrusive.

Crate: crates/capsem-tray/
Depends on: S12 (tray sprint, complete)

## Goals

1. **macOS native notifications** -- push `NSUserNotification` / `UNUserNotificationCenter` for VM events (provisioned, stopped, errored, suspended)
2. **Attention badge** -- overlay a small colored dot on the tray icon when user action is needed (e.g., VM waiting for input, sandbox policy prompt, corp config update)
3. **Pulse animation** -- subtle icon pulse/throb when there's an unacknowledged attention state, stops when user clicks the tray

## Architecture

```
capsem-gateway GET /status
  -> StatusResponse now includes:
     - notifications: Vec<Notification>    (new events since last poll)
     - attention: bool                     (user input needed)

capsem-tray main loop:
  -> on poll result:
     1. update menu + icon state (existing)
     2. fire macOS notifications for new events
     3. if attention=true: show badge dot + start pulse
     4. on tray click: clear pulse, mark attention acknowledged
```

### Notification Types

| Event | Notification | Badge |
|-------|-------------|-------|
| VM provisioned | "VM {name} ready" | No |
| VM stopped | "VM {name} stopped" | No |
| VM errored | "VM {name} failed: {reason}" | Yes |
| Sandbox policy prompt | "VM {name} needs approval" | Yes |
| Corp config update | "New configuration available" | Yes |
| Gateway reconnected | "Service reconnected" | No |

### Badge Dot

Overlay a small red/orange circle (6x6px) on the bottom-right corner of the tray icon. Composited at runtime by modifying the RGBA buffer before calling `set_icon()`. No extra PNG needed -- just paint pixels into the decoded icon buffer.

### Pulse Animation

When attention is needed, alternate between the normal icon and a slightly brighter/dimmed version every ~800ms. Uses a timer in the main loop (every N frames at 60Hz). Stops when the user clicks the tray (menu opens).

## Sub-sprints

### SN1: Gateway Notification Plumbing

Status: Not started

- [ ] Add `Notification` type to gateway/service: `{ id, kind, vm_id, message, timestamp }`
- [ ] Add `attention: bool` field to `StatusResponse`
- [ ] Add `notifications: Vec<Notification>` to `StatusResponse` (events since last poll, cleared on read)
- [ ] Service tracks pending notifications in memory (ring buffer, last 50)
- [ ] Service sets `attention=true` when a VM needs user input
- [ ] Add `POST /notifications/ack` to clear attention state

### SN2: macOS Native Notifications

Status: Not started

- [ ] Add `notify-rust` or use `objc2-user-notifications` for `UNUserNotificationCenter`
- [ ] Fire notification on each new event from `StatusResponse.notifications`
- [ ] Deduplicate: track last-seen notification ID, only fire new ones
- [ ] Notification click: open tray menu or launch UI for the relevant VM
- [ ] Respect macOS notification settings (user can disable in System Preferences)
- [ ] Unit tests for notification dedup logic

### SN3: Badge Dot Overlay

Status: Not started

- [ ] `icons.rs`: add `fn overlay_badge(rgba: &mut [u8], width: u32, height: u32)` that paints a 6x6 red circle at bottom-right
- [ ] New `TrayState::Attention` variant (or flag alongside existing states)
- [ ] When `attention=true` in poll result: composite badge onto current icon
- [ ] When `attention=false`: show normal icon (no badge)
- [ ] Unit test: badge overlay produces red pixels in expected region
- [ ] Unit test: badge doesn't corrupt the rest of the icon

### SN4: Pulse Animation

Status: Not started

- [ ] Track `pulse_active: bool` and `pulse_frame: u32` in main loop state
- [ ] When attention is set: toggle `pulse_frame` every ~50 loop iterations (~800ms at 60Hz)
- [ ] Pulse effect: alternate between normal icon+badge and dimmed icon+badge (reduce alpha by 30%)
- [ ] On menu open (any `MenuEvent` received): set `pulse_active = false`, restore normal icon
- [ ] Pulse stops immediately on tray click -- no lingering animation
- [ ] Keep CPU usage negligible (no extra redraws when not pulsing)

### SN5: Acknowledge Flow

Status: Not started

- [ ] Tray sends `POST /notifications/ack` when user clicks tray while attention is active
- [ ] Service clears attention state on ack
- [ ] If new attention event arrives after ack, badge reappears
- [ ] "Dismiss All" menu item when badge is showing (optional)

## Acceptance Criteria

- [ ] VM provision/stop/error fires macOS notification
- [ ] Notification shows VM name and event type
- [ ] Badge dot appears when attention is needed
- [ ] Badge disappears when user clicks tray
- [ ] Pulse animation visible and not jarring
- [ ] Pulse stops on tray click
- [ ] No CPU increase when idle (no badge, no pulse)
- [ ] Works with existing grey/black icon variants

## Depends On

- S12 tray sprint (complete)
- Gateway `/status` response extension (SN1)

## Reference

- Tray crate: `crates/capsem-tray/src/`
- Gateway status endpoint: `crates/capsem-gateway/src/`
- Service API types: `crates/capsem-service/src/api.rs`
- Icon assets: `graphics/icon/` (source), `crates/capsem-tray/icons/` (embedded)
- macOS notification API: `UNUserNotificationCenter` (macOS 10.14+)
