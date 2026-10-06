# Retain whole-room intent with exact furniture allocations

Add a bounded canonical RoomFurnitureHandoff that preserves the original room
geometry, exclusions and exact furniture constraints alongside selected items.
Its digest distinguishes different room goals with identical furniture plans.
Whole-room native dimension checks and fourteen passing semantic regressions
cover the existing compiler and allocator; native custody integration is separate.
