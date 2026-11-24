; diffr additions (MIT); later patterns override the grammar's.
; The TypeScript query makes every capitalized identifier a type; keep
; functions and constants what JavaScript's query says they are.
((identifier) @constant
 (#match? @constant "^[A-Z_][A-Z\\d_]+$"))

(function_declaration
  name: (identifier) @function)

(function_expression
  name: (identifier) @function)

(generator_function_declaration
  name: (identifier) @function)

(call_expression
  function: (identifier) @function)

(variable_declarator
  name: (identifier) @function
  value: [(function_expression) (arrow_function)])

; Angle brackets of type parameters are brackets, not comparisons.
(type_parameters
  "<" @punctuation.bracket
  ">" @punctuation.bracket)
