vim9script
# Vim9 appends by assigning just past the end of a List.
var L = [1]
L[1] = 2
echo L
try
  L[3] = 2
catch
  echo matchstr(v:exception, 'E\d\+')
endtry
# A compiled :def iterates by index.
def ForIndex(): list<any>
  var items = [1, 2, 3]
  for item in items
    remove(items, 0)
  endfor
  var appended = [1]
  var seen = []
  for item in appended
    seen->add(item)
    if len(appended) < 4
      appended->add(item + 1)
    endif
  endfor
  return [items, seen]
enddef
echo ForIndex()
# Script-level Vim9 loops follow the next item, like legacy script.
var M = [1, 2, 3]
for item in M
  remove(M, 0)
endfor
echo M
# Constant-false arms are skipped whatever the previous arm was.
def Skipped(): number
  if false
    NoSuchCommand
  elseif false
    NoSuchCommand
  endif
  return 7
enddef
echo Skipped()
def Middle(): number
  if false
    NoSuchCommand
  elseif true
    return 1
  elseif false
    NoSuchCommand
  else
    NoSuchCommand
  endif
  return 2
enddef
echo Middle()
# Ex commands compile by their own grammar.
def Cross()
  legacy echo len([1])
enddef
Cross()
def Unknown()
  nosuchcmd arg(1)
enddef
try
  defcompile Unknown
catch
  echo matchstr(v:exception, 'E\d\+')
endtry
# A Float after `..` is a Float literal, unlike legacy script.
echo 'a' .. 2.5 'a' .. 2.5e1 1 .. 2.5 'x' .. 1.0 .. 3 1.5 .. 'b'
var text = 'p'
try
  text ..= 0.5
catch
  echo matchstr(v:exception, 'E\d\+')
endtry
# `for name: type` declares and checks each item's type.
def TypedSum(): number
  var total = 0
  for item: number in [1, 2]
    total += item
  endfor
  return total
enddef
echo TypedSum()
var typed_total = 0
for typed_item: number in [3, 4]
  typed_total += typed_item
endfor
echo typed_total
try
  for wrong_item: number in [1, 'x']
    echo wrong_item
  endfor
catch
  echo matchstr(v:exception, 'E\d\+')
endtry
def TypedWrong()
  for w: string in [1, 2]
    echo w
  endfor
enddef
try
  defcompile TypedWrong
catch
  echo matchstr(v:exception, 'E\d\+')
endtry
def TypedPairs(): string
  var out = ''
  for [k: string, v: number] in [['a', 1], ['b', 2]]
    out ..= k .. v
  endfor
  return out
enddef
echo TypedPairs()
def TypedAny(): number
  var values: list<any> = [1, 'x']
  var count = 0
  for value: number in values
    count += value
  endfor
  return count
enddef
try
  echo TypedAny()
catch
  echo matchstr(v:exception, 'E\d\+')
endtry
