; diffr additions (MIT); later patterns override the grammar's.
; The grammar captures a whole key = value pair as a property; keep only
; the key.
(pair) @none
(pair
  (bare_key) @property)
(pair
  (dotted_key
    (bare_key) @property))
