# Recover externally paused game-clock runs

The bounded, selected-order and excavation clients can read a pending stop
when the game reports an external pause. The retained operation stays active
until the native owner verifies the raw pause or reports loss of the source.
