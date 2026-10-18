" :let, :unlet, :const and scopes.
let x = 1
let x += 2
let x -= 1
let x *= 10
let x /= 4
let x %= 3
echo x
let s = 'a'
let s .= 'b'
let s ..= 'c'
echo s
let [a, b] = [1, 2]
let [c, d; rest] = [1, 2, 3, 4]
echo a b c d rest
let l = [1, 2]
let l += [3]
let l[0] = 10
let l[1:2] = [20, 30]
echo l
let dict = {}
let dict.key = 'v'
let dict['other'] = 'w'
let dict.nested = {'deep': 1}
let dict.nested.deep += 5
echo dict.key dict.other dict.nested.deep
unlet dict.other
echo has_key(dict, 'other')
unlet l[0]
echo l
let g:global = 'g'
echo g:global global exists('g:global') exists('global')
unlet g:global
echo exists('g:global')
unlet! nosuchvariable
let var_1 = 'one'
let name = 'var'
echo {name}_1
let {name}_2 = 'two'
echo var_2
const c1 = 5
try
  let c1 = 6
catch
  echo 'const:' v:exception
endtry
let lines =<< trim END
    first
      second
    third
END
echo lines
let raw =<< MARK
  kept  indentation
MARK
echo raw
let &tabstop = 4
echo &tabstop &ts
let @a = 'register'
echo @a
let @a .= ' more'
echo @a
echo $HOME != ''
let v:errmsg = 'set'
echo v:errmsg
let list = [1, 2, 3]
lockvar list
try
  call add(list, 4)
catch
  echo 'locked:' v:exception
endtry
unlockvar list
call add(list, 4)
echo list
echo count
echo l:
