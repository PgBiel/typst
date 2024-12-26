--- test-colliders ---

#let c1 = block(inset: (top: 5pt, left: 0pt, bottom: 15pt, right: 5pt), rect(height: 1cm, width: 1cm, fill: red))
#let c2 = block(inset: (top: 0pt, left: 5pt, bottom: 5pt, right: 0pt), rect(height: 1cm, width: 1cm, fill: red))
#set par(colliders: (par.collider(left, -5pt, c1), par.collider(right, 3cm, c2),))

#set page(width: 4cm, height: auto)
#place(left, c1, dy: -5pt)
#place(right, c2, dy: 3cm)
#lorem(30)
