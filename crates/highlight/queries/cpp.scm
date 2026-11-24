; diffr additions (MIT); later patterns override the grammar's.
; Angle brackets of templates are brackets, not comparisons.
(template_parameter_list
  "<" @punctuation.bracket
  ">" @punctuation.bracket)
(template_argument_list
  "<" @punctuation.bracket
  ">" @punctuation.bracket)
(this) @variable.builtin
