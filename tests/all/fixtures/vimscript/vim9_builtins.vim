vim9script
var effects = 0
def WrongBuiltin()
  effects += 1
  echo str2nr(12)
enddef
try
  defcompile WrongBuiltin
catch
  echo 'builtin compile rejected' effects
endtry
try
  echo str2nr(12)
catch
  echo 'builtin runtime rejected'
endtry
def WrongCount()
  echo split()
enddef
try
  defcompile WrongCount
catch
  echo 'arity rejected'
endtry
def WrongCallback()
  var values: list<number> = [1, 2]
  filter(values, (i: number, value: number): string => 'yes')
enddef
try
  defcompile WrongCallback
catch
  echo 'callback rejected'
endtry
class Box
  var value: number
endclass
type BoxType = Box
def ClassAsValue()
  effects += 1
  var invalid = Box
enddef
try
  defcompile ClassAsValue
catch
  echo 'class rejected' effects
endtry
echo instanceof(Box.new(), Box, BoxType)
class InferredFields
  var amount = 1
endclass
class StrictConstructor
  def new(amount: number)
  enddef
endclass
def BadConstructor()
  effects += 1
  var invalid = StrictConstructor.new('bad')
enddef
try
  defcompile BadConstructor
catch
  echo 'constructor rejected' effects
endtry
echo InferredFields.new(7).amount
def WantsBool(Callback: func(number): bool): bool
  return Callback(1)
enddef
def InferredReturn(): bool
  return WantsBool((argument) => argument .. 'string')
enddef
try
  defcompile InferredReturn
catch
  echo 'inferred callback rejected'
endtry
for value in [1, 2]
  try
    value += 1
  catch
    echo 'loop binding retained' value
  endtry
endfor
