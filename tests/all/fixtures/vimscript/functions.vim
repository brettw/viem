" User functions, arguments, closures and funcrefs.
function! Add(a, b)
  return a:a + a:b
endfunction
echo Add(2, 3)
function! Varargs(first, ...)
  return [a:first, a:0, a:000, a:0 > 0 ? a:1 : 'none']
endfunction
echo Varargs(1) Varargs(1, 2, 3)
function! Defaults(a, b = 10, c = a:b + 1)
  return [a:a, a:b, a:c]
endfunction
echo Defaults(1) Defaults(1, 2) Defaults(1, v:none, 5)
function! Locals()
  let x = 'local'
  let l:y = 'explicit'
  return x .. ' ' .. y .. ' ' .. string(sort(keys(l:)))
endfunction
echo Locals()
function! s:Private()
  return 'script-local'
endfunction
echo s:Private() <SID>Private()
function! Recurse(n)
  return a:n <= 1 ? 1 : a:n * Recurse(a:n - 1)
endfunction
echo Recurse(10)
let obj = {'value': 42}
function! obj.get() dict
  return self.value
endfunction
echo obj.get()
function! s:Getter() dict
  return self.name
endfunction
let person = {'name': 'Ann', 'Get': function('s:Getter')}
echo person.Get()
function! MakeCounter()
  let count = 0
  function! Increment() closure
    let count += 1
    return count
  endfunction
  return funcref('Increment')
endfunction
let Counter = MakeCounter()
echo Counter() Counter()
function! Outer(x)
  return {y -> a:x + y}
endfunction
echo Outer(10)(5)
let Partial = function('Add', [100])
echo Partial(1) call('Add', [3, 4]) call(Partial, [2])
echo function('Add')(1, 1)
echo exists('*Add') exists('*NoSuch') exists('*strlen')
delfunction Add
echo exists('*Add')
function! Abort() abort
  call Undefined()
  echo 'not reached'
endfunction
call Abort()
echo 'after abort'
function! NoAbort()
  call Undefined()
  echo 'continues'
endfunction
call NoAbort()
function! TooFew(a, b)
endfunction
call TooFew(1)
call TooFew(1, 2, 3)
function! Range() range
  return a:firstline .. '-' .. a:lastline
endfunction
function! Dup()
  return 1
endfunction
function Dup()
  return 2
endfunction
echo Dup()
echo map([1, 2], function('Recurse'))
echo sort([3, 1, 2], {a, b -> b - a})
function! Compare(a, b)
  return a:a - a:b
endfunction
echo sort([3, 1, 2], 'Compare')
