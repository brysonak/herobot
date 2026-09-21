# herobot commands

## Moderation

### `/warn <user> [reason]`
`Moderate Members`

Records a warning and DMs the user. The reply gives the case number and how many
records the user now has. If the DM fails (closed DMs), the warning still stands
and the reply says so.

Reason is optional so you can act first. Fill it in later with `/case reason`.

### `/note <user> <text>`
`Moderate Members`

Attaches a note to a user's record. No DM, no notification, nothing visible to
the user. Use it for context that isn't a punishment: an alt account you've
linked, a warning another mod gave verbally, a pattern you want the next mod to
know about.

### `/mute <user> <duration> [reason]`
`Moderate Members`

Times the user out. They can still read; they can't send, react, join threads or
speak in voice.

Duration is required and capped at 28 days by Discord. Accepts `30m`, `2h`,
`1d12h`, `45s`, `2w`, or a bare number meaning minutes

A timed-out user who leaves and rejoins loses the timeout. That's Discord
behaviour, not a bug here.

### `/unmute <user>`
`Moderate Members`

Lifts a timeout early.

### `/kick <user> [reason]`
`Kick Members`

DMs the user, then kicks. The DM goes first because after a kick there may be no
shared server left to message through.

### `/ban <user> [reason] [duration] [purge_days]`
`Ban Members`

Omit `duration` for a permanent ban. With a duration (`7d`, `12h`, `1d12h`), the
ban lifts automatically

`purge_days` (0-7) deletes that many days of the user's messages as part of the
ban. Defaults to 0.

Works on users who aren't in the server.

### `/softban <user> [reason] [purge_days]`
`Ban Members`

Ban then immediately unban. Wipes the user's recent messages without keeping them out. `purge_days` defaults to 1.

### `/unban <user_id>`
`Ban Members`

Takes a raw user ID, since a banned user can't be picked from a list. Also marks
any pending temp ban as lifted so the sweeper stops retrying it.

## Cases

Every moderation action writes one row and gets a case number. Warnings, notes,
mutes, kicks, bans and automatic actions all share the same numbering, so a case
number identifies exactly one event.

### `/case view <id>`
`Moderate Members`

Full detail on one case: who, what, which moderator, when, reason, duration, and
whether a temp punishment has been lifted.

### `/case reason <id> <reason>`
`Moderate Members`

Sets or replaces a case's reason.

Note that Discord's own audit log entry keeps whatever reason it was given at the
time, there's no API to edit it. After amending, the bot's record and the audit
log will disagree.

### `/case delete <id>`
`Moderate Members`

Removes one case. The full contents are written to the log channel first.

### `/records <user>`
`Moderate Members`

That user's 25 most recent cases, newest first, with case numbers.

### `/cases [kind] [moderator] [search] [limit]`
`Moderate Members`

Browse across everyone. All filters optional and they combine.

- `kind` - dropdown: warn, note, mute, kick, ban, softban, invite, image spam
- `moderator` - everything one mod did
- `search` - substring match on the reason, wildcards escaped
- `limit` - 1 to 50, default 20

`/cases kind:ban limit:50` for "what happened during that raid".
`/cases search:alt` to find every case mentioning alts.

### `/clearrecords <user>`
`Ban Members`

Wipes a user's entire history. Everything destroyed is written to the log channel
first, including cases beyond the 25 that `/records` displays.

## Channels

### `/purge <scan> [user] [pattern] [links] [bots]`
`Manage Messages`

`scan` is how many recent messages to examine (1-500), not how many to delete.
The filters decide what actually goes; with no filters, everything in range goes.

- `user` - only that user's messages
- `pattern` - regex, case-insensitive
- `links` - only messages with links, embeds or attachments
- `bots` - only messages from bots

Pinned messages are always skipped. Anything older than 14 days is skipped and
counted separately in the reply, because Discord refuses to bulk-delete it.

The reply is ephemeral. The log channel gets a permanent entry with the filters
used and the count.

### `/slowmode <seconds>`
`Manage Channels`

0 disables. Maximum 21600 (6 hours).

### `/lock` and `/unlock`
`Manage Channels`

Denies or clears `Send Messages`, `Send Messages in Threads` and `Add Reactions`
for `@everyone` in the current channel.

`/unlock` resets the overwrite to neutral. If the channel had those permissions
denied before you locked it, unlocking leaves it more open than it started.

## Tickets

Tickets are channels under the ticket category, named `userissue-N` or `bug-N`
with separate counters.

### `/ticket create <category>`
`Anyone can run this`

Opens a private channel for the user and the category's staff role, pinging that
role unless pings are off. The first message has **Close** and **Close with
Reason** buttons, usable by `Moderate Members` only.

### `/ticket add <user>` and `/ticket remove <user>`
`Moderate Members`

Grants or revokes access to the current ticket. Remove uses a deny overwrite, so
it also works on people who can see the ticket through a role. Administrators
can't be removed.

### `/ticket close`
`Moderate Members`

Deletes the ticket and DMs everyone who spoke in it, plus the opener, a summary:
type, opener, closer, open and close times, reason, participants.

### `/ticket pings <enabled>`
`Moderate Members`

Toggles the staff role ping on new tickets. Works from any channel.

## Information

### `/whois <user>`
`Moderate Members`

Account ID, creation date, join date, nickname, roles, record count, and time
remaining if they're currently muted. Works on users who have left the server,
with reduced detail.

## Notes
Reasons are capped at 512 characters.

Durations accept `s`, `m`, `h`, `d`, `w` and combinations like `1d12h`. A bare
number means minutes. A number with a trailing unit missing (`1h30`) is rejected
