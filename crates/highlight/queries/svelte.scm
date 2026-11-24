; diffr additions (MIT), after tree-sitter-html's and the grammar's queries.
; Block tags ({#if}, {:else}, {/if}, {@html}) are keywords.
(block_start_tag) @keyword
(block_tag) @keyword
(block_end_tag) @keyword
(block_start_tag "#" @keyword)
(block_tag ":" @keyword)
(block_end_tag "/" @keyword)
