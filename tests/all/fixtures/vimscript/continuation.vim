" Line continuation and comments.
let list = [
      \ 1,
      \ 2,
      "\ a comment inside the continuation
      \ 3,
      \ ]
echo list
let dict = {
      \ 'a': 1,
      \ 'b': 2
      \ }
echo dict.a dict.b
echo 'a'
      \ . 'b'
      \ . 'c'
let x = 1 " trailing comment
echo x
if x == 1 " comment after if
  echo 'yes'
endif " comment after endif
" a whole-line comment
  " an indented comment
echo 'end'
