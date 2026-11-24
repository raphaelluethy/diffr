; Written for diffr (MIT), as tree-sitter-php's own injections-text.scm:
; the text around PHP blocks is HTML.
((text) @injection.content
  (#set! injection.language "html")
  (#set! injection.combined))
