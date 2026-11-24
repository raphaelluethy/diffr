; diffr additions (MIT); later patterns override the grammar's.

; A heading is a heading, its markers included.
(atx_heading) @markup.heading
(setext_heading) @markup.heading

; Nodes an injection starts at must not start a highlight too: the
; highlighter can emit the two starts in the wrong order. These captures
; are recognized by no name, so they only cancel the grammar's. (The
; grammar's `(code_fence_content) @none` is removed from its query text.)
(atx_heading
  (inline) @injected)
(setext_heading
  (paragraph) @injected)

; A fenced block's fences and info string are literal; its content is the
; injected language's, plain when the language is unknown, literal when the
; block names none.
(fenced_code_block) @none
(fenced_code_block_delimiter) @text.literal
(info_string) @text.literal
(fenced_code_block
  .
  (fenced_code_block_delimiter)
  .
  (block_continuation)
  .
  (code_fence_content) @text.literal)
