; diffr additions (MIT); later patterns override the grammar's.
; $x is a variable, sigil and all, as in Chroma.
(variable_name
  "$" @variable)
[
  (php_tag)
  (php_end_tag)
] @preproc
