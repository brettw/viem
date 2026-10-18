" :execute, execute(), :eval and string building.
let name = 'x'
execute 'let ' . name . ' = 42'
echo x
execute "if 1 | echo 'exec if' | endif"
execute "let a = 1\nlet b = 2"
echo a b
execute 'echo' '"joined"' 1
let output = execute('echo "captured"')
echo string(output)
let output = execute(['echo 1', 'echo 2'])
echo string(output)
eval [1, 2]->add(3)
let l = [1]
eval l->add(2)
echo l
function! ExecInFunction()
  let local = 'L'
  execute 'let local2 = local . "2"'
  return local2
endfunction
echo ExecInFunction()
execute 'echo "a" |' 'echo "b"'
