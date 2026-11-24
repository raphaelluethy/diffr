; diffr additions (MIT); later patterns override the grammar's.
; Numbers are numbers, booleans language constants (the grammar calls all
; literals constant.builtin).
(boolean_literal) @boolean
(integer_literal) @number
(float_literal) @number

; SCREAMING_CASE constants (the grammar's regex has a stray quote).
((identifier) @constant
 (#match? @constant "^[A-Z][A-Z\\d_]+$"))

; A lifetime is one token.
(lifetime "'" @label)
(lifetime (identifier) @label)
