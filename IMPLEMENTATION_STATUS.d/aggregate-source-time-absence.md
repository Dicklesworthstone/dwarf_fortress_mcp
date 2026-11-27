# Source-qualified aggregate absence

The operational aggregate executor uses the world query presence classifier for
group keys and numeric metric exclusions. A retained snapshot evaluates explicit
absence at its own tick even when the current session has advanced. Queries for
a future observation and queries for a known absence therefore agree with the
resulting group labels.

A regression exercises scalar and tuple groups, metrics and the presence filter
at two ticks through current and retained execution. An earlier helper assertion
that treated future absence as established has been corrected. Source reviewed
via GitHub. Rust execution and full qualification remain pending.
