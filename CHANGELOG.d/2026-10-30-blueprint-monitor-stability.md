## Preserve whole-blueprint observation continuity

Reset shared stability after changed captures at one game tick, and require a
later advancing tick before a correction or restarted owner can seed a new
streak. Add local withdrawal after unfinished response publication, durable
continuity reset before another read, and preservation of consumed evidence and
clock floors without allowing an abandoned attempt to publish progress.

Four regression groups accompany the change. Their final run was blocked by
SIGKILL during complete adapter test compilation; no new test execution or full
runtime/native/live qualification is claimed at this source checkpoint.
