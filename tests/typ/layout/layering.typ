// Tests overlay and underlay

---
#set page(width: 250pt, height: 150pt)
// Displays a yellow square on the bottom; a red, tilted square on the middle;
// 'Hey!' on the top
#overlay(
    rect(width: 100pt, height: 100pt, fill: yellow),
    rotate(45deg, rect(width: 100pt, height: 100pt, fill: red)),
    align(center + horizon)[Hey!]
)

---
#set page(width: 250pt, height: 250pt)
// Displays a yellow square on the top left;
// a red square on the middle;
// a blue square on the bottom right
#underlay(
    rect(width: 100pt, height: 100pt, fill: yellow),
    move(dx: 50%, dy: 50%, rect(width: 100%, height: 100%, fill: red)),
    move(dx: 100%, dy: 100%, rect(width: 100%, height: 100%, fill: blue))
)

---
#set page(width: 250pt, height: 12em)
// Displays an empty table on the bottom; a tilted square on the middle;
// large red "CLASSIFIED" on the top
#overlay(
  table(columns: (2em,)*5, rows: (2em,)*4),
  rotate(-45deg, align(center+horizon, box(width: 70%, height: 70%, stroke: red + 1pt))),
  place(center+horizon, rotate(-45deg, text(red, 12pt)[*CLASSIFIED*]))
)

---
#set page(width: 250pt, height: 12em)
// Displays a yellow background under the given text
And so, the young boy #underlay([announced that he], box(width: 100%, height: 100%, fill: yellow))
would leave.
