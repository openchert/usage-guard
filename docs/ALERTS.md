# Alert Model

UsageGuard treats alerts as a core desktop feature.

- Native OS notifications are the primary delivery mechanism.
- The widget also keeps a visible alert state on the affected provider card until the alert clears.
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

## API and admin monitoring alerts

Built-in API/admin monitoring sources keep the existing non-consumer alerts:

- near budget limit
- budget exceeded
- under-used / inactivity reminder

These are preserved so organization monitoring still surfaces spend pressure even though it does not use the consumer quota-window model.

## Delivery and re-arm behavior

- Each alert is tracked independently per provider account.
- Consumer quota alerts use the reset timestamp as part of the notification signature so the same alert can fire again after a new quota window starts.
- Alerts also re-arm after they clear and later become active again.
- Demo snapshots never emit notifications.

## Widget behavior

- active alerts prepend summary lines to the card tooltip
- card borders are tinted by highest active severity
- a small badge appears on the card while any alert is active
- consumer status cards stay non-alerting until a real quota window is available

Severity order:

- `critical`
- `warning`
- `info`
