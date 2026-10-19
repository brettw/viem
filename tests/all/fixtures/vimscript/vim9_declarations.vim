vim9script
const g:ViemReadOnly = 3
final g:ViemShallow = [1]
add(g:ViemShallow, 2)
try
  var s:compatibility = 4
catch
  echo 'namespace rejected'
endtry
var compatibility = 4
echo g:ViemReadOnly g:ViemShallow compatibility

def Scoped()
  const g:ViemInner = 5
enddef
Scoped()
echo g:ViemInner

def Reserved()
  var true = 0
enddef
try
  defcompile Reserved
catch
  echo 'reserved rejected'
endtry
def MissingType()
  var missing
enddef
try
  defcompile MissingType
catch
  echo 'missing type rejected'
endtry
def VoidVariable()
  var nothing: void
enddef
try
  defcompile VoidVariable
catch
  echo 'void type rejected'
endtry
def BadBlocks()
  if false
    while true
  endif
enddef
try
  defcompile BadBlocks
catch
  echo 'block pairing rejected'
endtry
def LoopReturn(): number
  while true
    return 1
  endwhile
enddef
try
  defcompile LoopReturn
catch
  echo 'loop return rejected'
endtry

def LiteralBody(): list<string>
  var lines =<< trim END
    enddef
    endclass
    function
  END
  return lines
enddef
echo LiteralBody()

def NestedLegacy(): number
  function Local(n)
    return a:n + 1
  endfunction
  return Local(2)
enddef
echo NestedLegacy()

var outer = 0
try
  for outer in [1]
  endfor
catch
  echo 'loop shadow rejected' outer
endtry
g:ViemLoop = 0
for g:ViemLoop in [1, 2]
endfor
echo g:ViemLoop
