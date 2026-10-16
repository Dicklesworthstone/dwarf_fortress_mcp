- Add bounded whole-request automatic room placement using deterministic
  minimum-remaining-domain backtracking and the existing connected RoomPlan
  compiler. Preserve all original hard constraints and explicit origin choices;
  retain canonical search evidence and distinguish exhausted from inconclusive
  search without emitting partial plans. All 16 actual Python component tests
  pass; native/Rust/full-repository qualification remains separate.
