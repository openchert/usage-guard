# Alert Model

UsageGuard treats alerts as a core desktop feature.

- Native OS notifications are the primary delivery mechanism.
- The widget also keeps a visible alert state on the affected connection card until the alert clears.
- Quiet hours still suppress non-critical notifications, but the widget state remains visible.

## Consumer quota alerts

Local consumer sources expose up to two quota windows:

- `5h`
- `week`

Current acquisition model:

- Codex alerts use the freshest available local consumer quota source: recent session logs first, then the short-lived token-backed fallback sourced from `~/.codex/auth.json`.
- Claude Code alerts use the normalized local consumer snapshot sourced from `~/.claude/.credentials.json` plus the built-in quota fetch when a valid local token is available.
- Status-only consumer snapshots do not emit quota alerts until an actual quota window is present.

UsageGuard evaluates two alert types on every available consumer window.

### Near exhaustion

- `5h`: alert at `>= 90%` used
- `week`: alert at `>= 80%` used
- fully exhausted quota escalates to a `critical` alert

### Use before reset

- `5h`: alert when reset is within `45 minutes` and usage is `<= 20%`
- `week`: alert when reset is within `24 hours` and usage is `<= 40%`
- reminders are skipped if the provider does not supply a valid reset timestamp

## Delivery and re-arm behavior

- Each alert is tracked independently per local connection.
- Notification dedup memory is kept in-process only and resets when the app exits.
- Consumer quota alerts use the normalized reset timestamp as part of the notification signature so the same alert can fire again after a new quota window starts.
- The same alert does not re-notify on refresh, even if it temporarily clears and then returns in the same app session.
- A different alert on the same connection re-arms notification delivery for later alerts on that card.
- Quiet-hours-suppressed alerts are not remembered until they are actually emitted.

## Widget behavior

- active alerts prepend summary lines to the card tooltip
- card borders are tinted by highest active severity
- a small badge appears on the card while any alert is active
- consumer status cards stay non-alerting until a real quota window is available

Severity order:

- `critical`
- `warning`
- `info`
