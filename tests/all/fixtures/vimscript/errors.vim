" Errors are reported and the script continues with the next line.
echo undefined_variable
echo 'after undefined'
notacommand
echo 'after unknown command'
echo 'x' | call NoSuchFunction() | echo 'same line'
echo 'next line'
let x = [1, 2][5]
let d = {}
echo d.missing
call add(1, 2)
echo 1 + [1]
echo "unterminated
if 1
  echo 'if body'
endwhile
endif
endfor
let = 5
echo 'done'
