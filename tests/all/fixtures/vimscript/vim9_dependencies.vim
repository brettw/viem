vim9script
var effects = 0
def Inner(): number
  return 'wrong'
enddef
def Outer(): number
  effects += 1
  return Inner()
enddef
try
  defcompile Outer
catch
  echo 'effects:' effects
endtry
def Recursive(n: number): number
  if n == 0
    return 0
  endif
  return Recursive(n - 1) + 1
enddef
echo Recursive(3)
