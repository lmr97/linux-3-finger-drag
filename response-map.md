# Response States

Under the conditions `dragEndDelay > 0` and `dragEndDelayCancellable == true`.

Gesture Type | Subtype | Finger Count | Mouse State | Mouse-up Timer Running?
--- | --- | --- | --- | ---
`Hold`  | `Begin`  | 3 | down | no
`Hold`  | `End`    | 3 | down | yes
`Hold`  | `!(End \|\| Begin)` | (any) | up | no
`Swipe` | `Begin`  | 3 | down | no
`Swipe` | `End`    | 3 | down | yes
`Swipe` | `Update` | 3 | down | no
`Swipe` | `!(End \|\| Begin \|\| Update)` | (any) | up | no
`!(Hold \|\| Swipe)` | (any) | 1 or 2 or 4 | up | no