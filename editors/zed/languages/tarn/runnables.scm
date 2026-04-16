; Top-level `name:` scalar — click the gutter runnable to run the whole file.
; Scoped to the outermost block_mapping so nested `name:` keys inside
; `request:` / step blocks do not get flagged.
((document
  (block_node
    (block_mapping
      (block_mapping_pair
        key: (flow_node (plain_scalar (string_scalar) @_key))
        (#eq? @_key "name")) @run)))
  (#set! tag tarn-file))
