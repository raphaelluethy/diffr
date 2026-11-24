; diffr additions (MIT); later patterns override the grammar's.
; The grammar's query ends with a catch-all `(variable) @type` and colors
; whole signatures as functions; as in Chroma, variables are plain and only
; top-level definitions are functions.
(variable) @variable
(decl/signature) @none
(declarations
  (decl/signature
    name: (variable) @function))
(declarations
  (decl/function
    name: (variable) @function))
(declarations
  (decl/bind
    name: (variable) @function))
((constructor) @boolean
  (#any-of? @boolean "True" "False"))
