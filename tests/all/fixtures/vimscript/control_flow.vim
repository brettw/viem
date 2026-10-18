" Conditionals and loops.
let n = 5
if n > 10
  echo 'big'
elseif n > 3
  echo 'medium'
else
  echo 'small'
endif
if 0 | echo 'never' | else | echo 'else branch' | endif
if 1
  if 0
    echo 'no'
  else
    echo 'nested else'
  endif
endif
let i = 0
while i < 3
  let i += 1
  if i == 2 | continue | endif
  echo 'while' i
endwhile
let i = 0 | while i < 3 | let i += 1 | endwhile | echo 'single line' i
for item in [1, 2, 3, 4]
  if item == 3 | break | endif
  echo 'for' item
endfor
for [k, v] in [['a', 1], ['b', 2]]
  echo k v
endfor
for [first; others] in [[1, 2, 3]]
  echo first others
endfor
for ch in 'héy'
  echon ch '.'
endfor
echo ''
for byte in 0z0102
  echo byte
endfor
let total = 0
for x in range(1, 100)
  let total += x
endfor
echo total
let list = [1, 2, 3]
for x in list
  if x == 1
    call add(list, 4)
  endif
endfor
echo list
if 0
  echo undefined_variable
  call NoSuchFunction()
  for x in nothing
  endfor
endif
echo 'skipped block fine'
let i = 0
while 1
  let i += 1
  try
    if i > 2 | break | endif
  finally
    echo 'finally' i
  endtry
endwhile
