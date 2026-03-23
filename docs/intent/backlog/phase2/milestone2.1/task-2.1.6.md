---
id: "2.1.6"
title: "Persist cues per song"
status: pending
crate: "rifflab-cue"
requirement: "FR-M6-06"
---

Save and load `Vec<Cue>` to and from `library/songs/{id}/cues.json`. Serialize cues using serde_json. On song load, read the cues file if present and populate CueEngine. On cue add/edit/delete, write the updated cue list back to disk. Handle missing or malformed cue files gracefully by starting with an empty cue list.
