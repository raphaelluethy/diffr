; diffr additions (MIT); later patterns override the grammar's.
(Attribute (Name) @attribute)
; CDATA sections are preprocessor-like, as in Chroma.
[
  (CDStart)
  (CData)
  "]]>"
] @preproc
