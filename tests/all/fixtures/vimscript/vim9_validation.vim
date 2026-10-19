vim9script
var text = "e\u0301x"
echo text[0] text[1] text[0 : 0]
var number = 1
++number
--number
echo number

def Destructure(): string
  var [x: number, y: string] = (4, 'four')
  var total = 0
  for [a, b] in [[1, 2], [3, 4]]
    total += a + b
  endfor
  return y .. ':' .. (x + total)
enddef
echo Destructure()

def FeatureGuard(): number
  if has('viem_impossible_feature') || exists_compiled('g:viem_impossible_variable')
    NoSuchCommand
  endif
  return 3
enddef
defcompile FeatureGuard
echo FeatureGuard()

def Nothing()
enddef
try
  var unused = Nothing()
catch
  echo 'void rejected'
endtry

def WrongDefault(value: number = 'wrong')
enddef
try
  defcompile WrongDefault
catch
  echo 'default rejected'
endtry

def WrongMember()
  var values: list<number> = [1]
  values[0] = 'wrong'
enddef
try
  defcompile WrongMember
catch
  echo 'member rejected'
endtry

class Immutable
  public final shallow: list<number>
  public const deep: list<number>
  def new(values: list<number>)
    this.shallow = copy(values)
    this.deep = copy(values)
  enddef
endclass
var immutable = Immutable.new([1])
add(immutable.shallow, 2)
try
  add(immutable.deep, 2)
catch
  echo 'const rejected'
endtry
echo immutable.shallow immutable.deep

echo len(null_string) len(null_tuple) tuple2list(null_tuple)

def Bars(flag: bool): number
  if flag | return 4 | else | return 5 | endif
enddef
echo Bars(false) | # a comment after the last command

defcompile Immutable

type Integers = list<number>
echo type(Integers) typename(Integers) string(Integers)
try
  var alias_value = Integers
catch
  echo 'alias value rejected'
endtry
echo null_list is null_list null_dict is null_dict null_blob is null_blob
echo copy(null_list) == null copy(null_dict) == null copy(null_blob) == null
var null_bytes = null_blob
try
  add(null_bytes, 1)
catch
  echo 'null blob rejected'
endtry
