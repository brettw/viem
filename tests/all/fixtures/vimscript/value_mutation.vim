" Locks, in-place map() and filter(), :for over a changing List,
" deepcopy() and List index bounds.
function! Try(command)
  try
    execute a:command
    return 'ok'
  catch
    return matchstr(v:exception, 'E\d\+')
  endtry
endfunction

" An item lock stops replacing that item; a List lock stops adding.
let g:L = [1, 2]
lockvar g:L[0]
echo Try('let g:L[0] = 8') Try('let g:L[1] = 9') Try('call add(g:L, 3)') g:L
unlockvar! g:L
let g:L = [1, 2]
lockvar 1 g:L
echo Try('let g:L[0] = 8') Try('call add(g:L, 3)') Try('let g:L = []') Try('call map(g:L, "v:val + 1")') Try('call filter(g:L, "1")') Try('call sort(g:L)') Try('call reverse(g:L)') g:L
unlockvar! g:L
let g:L = [1, [2]]
lockvar g:L
echo Try('let g:L[0] = 8') Try('call add(g:L[1], 3)') Try('let g:L[1][0] = 5') Try('call map(g:L, "v:val")') g:L
unlockvar! g:L
let g:L = [1, [2]]
lockvar 3 g:L
echo Try('let g:L[1][0] = 5') g:L
unlockvar! g:L
let g:D = {'a': 1}
lockvar g:D
echo Try('call map(g:D, "v:val + 1")') Try('let g:D.a = 2') Try('let g:D.b = 2') Try('call filter(g:D, "0")') g:D
unlockvar! g:D
let g:D = {'a': 1}
lockvar 1 g:D
echo Try('call map(g:D, "v:val + 1")') Try('let g:D.a = 5') Try('let g:D.b = 2') Try('call filter(g:D, "0")') g:D
unlockvar! g:D
let g:D = {'a': 1, 'b': 2}
lockvar g:D.a
echo Try('call map(g:D, "v:val + 10")') g:D
let g:B = 0z0102
lockvar g:B
echo Try('call remove(g:B, 0)') Try('call add(g:B, 3)') Try('call insert(g:B, 3)') Try('let g:B[0] = 9') Try('call filter(g:B, "1")') g:B
unlockvar! g:B
" map() stops at a locked item, keeping earlier results; locks move with items.
let g:L = [1, 2, 3]
lockvar g:L[1]
echo Try('call map(g:L, "v:val * 10")') g:L
unlockvar g:L[1]
echo Try('let g:L[1] = 0') g:L
let g:L = [1, 2, 3]
lockvar g:L[2]
call sort(g:L, 'N')
call reverse(g:L)
echo Try('let g:L[0] = 7') Try('let g:L[1] = 7') g:L
unlockvar! g:L

" map() and filter() change the List as each callback returns.
let L = [1, 2, 3]
call map(L, 'v:val + L[0]')
echo L
let g:M = [3, 2, 1]
call map(g:M, {i, v -> v + get(g:M, i - 1, 0)})
echo g:M
let L = [1, 2, 3]
call filter(L, 'v:val != 2 && len(L) > 0')
echo L
let g:M = [1, 2, 3, 4]
echo Try("call map(g:M, 'v:key == 1 ? nosuch : v:val * 2')") g:M

" A legacy :for follows the next item, not an index.
let L = [1, 2, 3]
for item in L
  call remove(L, 0)
endfor
echo L
for [setup, change] in [['[1]', 'if len(L) < 4 | call add(L, item + 1) | endif'],
      \ ['[1, 2, 3, 4]', 'if item == 1 | call remove(L, 1) | endif'],
      \ ['[1, 2, 3]', 'if item == 1 | call insert(L, 9, 1) | endif'],
      \ ['[1, 2, 3]', 'if item == 1 | call reverse(L) | endif'],
      \ ['[3, 1, 2]', 'if item == 3 | call sort(L) | endif'],
      \ ['[1, 2, 3]', 'if item == 1 | unlet L[1:] | endif']]
  execute 'let L = ' .. setup
  let seen = []
  for item in L
    call add(seen, item)
    execute change
  endfor
  echo seen L
endfor

" deepcopy(): noref copies every occurrence; deep or cyclic values fail.
let child = [1]
let L = deepcopy([child, child], 1)
let L[0][0] = 8
echo L
let L = deepcopy([child, child])
let L[0][0] = 8
echo L[1][0]
let d = {'x': 1}
let D = deepcopy({'a': d, 'b': d}, 1)
let D.a.x = 5
echo D
let g:C = [1]
call add(g:C, g:C)
echo Try('let g:X = deepcopy(g:C, 1)') len(g:C)
let X = deepcopy(g:C)
echo X[1] is X
let X[1] = 0
let g:C[1] = 0
let g:N = [0]
for i in range(150)
  let g:N = [g:N]
endfor
echo Try('let g:Y = deepcopy(g:N)')

" Assigning just past the end is an error in legacy script.
let g:M = [1]
echo Try('let g:M[1] = 2') g:M
let B = 0z01
let B[1] = 2
echo B
