" Legacy data semantics used by real configuration functions.
let amount = 6
let values =<< trim eval END
    value {amount * 7}
      nested {{literal}} {toupper('ok')}
END
echo values
let kept =<< trim END
    {amount} remains literal
      \ continued-looking text
END
echo kept
let nested = [[1], {'x': [2]}]
lockvar 1 nested
call add(nested[0], 3)
echo nested
try
  call add(nested, 4)
catch
  echo matchstr(v:exception, 'E\d\+')
endtry
unlockvar 1 nested
lockvar! nested
try
  let nested[1].x[0] = 5
catch
  echo matchstr(v:exception, 'E\d\+')
endtry
unlockvar! nested
let nested[1].x[0] = 5
echo nested
function! Bound(prefix, value) dict
  return a:prefix .. ':' .. self.label .. ':' .. a:value
endfunction
let object = {'label': 'first'}
let Partial = function('Bound', ['pre'], object)
echo Partial('value')
let object.label = 'changed'
echo call(Partial, ['again'])
function! Compare(a, b) dict
  return self.direction * (a:a - a:b)
endfunction
echo sort([3, 1, 2], 'Compare', {'direction': -1})
let source = [1, 2, 3]
let seen = []
for item in source
  call add(seen, item)
  if item == 1
    call remove(source, 1)
    call add(source, 4)
  endif
endfor
echo seen source
let name = 'blue'
let g:item_{name} = 42
echo g:item_blue exists('g:item_{name}') exists('g:item_missing')
echo exists('&tabstop') exists('+tabstop') exists('&definitely_not_an_option')
echo exists(':if') exists(':s') exists('*strlen') exists('*DefinitelyMissing')
let output = execute("echo 'captured' | echon ' text'")
echo string(output)
echo string(execute("silent! echo missing_name | echo 'tail'", 'silent!'))

let cycle = []
call add(cycle, cycle)
lockvar! cycle
unlockvar! cycle
call add(cycle, 'ok')
echo len(cycle) cycle[1]

let self_list = []
call add(self_list, self_list)
let self_dict = {}
let self_dict.self = self_dict
let shared = [1]
let mixed_cycle = [1]
call add(mixed_cycle, mixed_cycle)
echo typename(self_list) typename(self_dict) typename([shared, shared]) typename(mixed_cycle)
for blob_range in [[2, 0], [0, 99], [1, 1], [0, -1], [-2, -1], [0, 3], [0, -4]]
  let blob = 0z010203
  try
    let removed = call('remove', [blob] + blob_range)
    echo string(blob_range) string(removed) string(blob)
  catch
    echo string(blob_range) v:exception string(blob)
  endtry
endfor
