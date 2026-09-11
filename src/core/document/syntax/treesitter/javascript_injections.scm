; Explicit host profile for tagged templates, regular expressions and JSDoc.
(call_expression
  function: [
    (identifier) @injection.language
    (member_expression property: (property_identifier) @injection.language)
  ]
  arguments: (template_string (string_fragment) @injection.content)
  (#set! injection.include-children))
((regex_pattern) @injection.content
 (#set! injection.language "regex"))
((comment) @injection.content
 (#match? @injection.content "^/\\*\\*")
 (#set! injection.language "jsdoc"))
