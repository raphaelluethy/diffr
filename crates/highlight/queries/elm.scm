; diffr additions (MIT); later patterns override the grammar's.
; Parameters are plain, not functions.
(function_declaration_left
  (lower_pattern) @variable)
(union_variant
  (upper_case_identifier) @constructor)
(char_constant_expr) @string
