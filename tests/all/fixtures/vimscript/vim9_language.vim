vim9script
# Recorded with scripts/vimscript-oracle.sh; no user configuration or plugins.
type Numbers = list<number>
var numbers: Numbers = [2, 3]
final alias = numbers
add(alias, 4)
echo numbers

def Sum(values: Numbers, bias: number = 1): number
  var result = bias
  for value in values
    result += value
  endfor
  return result
enddef
echo Sum(numbers)

def Identity<T>(value: T): T
  var copy: T = value
  return copy
enddef
echo Identity<number>(4) Identity<string>('four')

def MakeEcho<T>(): func(T): T
  return (value: T): T => value
enddef
var EchoNumber = MakeEcho<number>()
echo EchoNumber(123)

var pair: tuple<number, string> = (7, 'seven')
echo pair pair[1] len(pair) tuple2list(pair)
for item in pair
  echo item
endfor

echo !'yes' !![] !![1, 2] 'value=' .. true

var messages: list<string> = []
def Record(message: string)
  add(messages, message)
enddef
def Work(): number
  defer Record('first')
  defer Record('second')
  return 42
enddef
echo Work() messages

interface HasValue
  def Value(): number
endinterface
class Base implements HasValue
  var value: number = 2
  def new(this.value)
  enddef
  def Value(): number
    return this.value
  enddef
endclass
class Child extends Base
  def Value(): number
    return super.Value() + 1
  enddef
endclass
var object: HasValue = Child.new(5)
echo object.Value()
enum Color
  Red, Green, Blue
endenum
echo Color.Green.name Color.Green.ordinal len(Color.values)
var unset_list = null_list
try
  add(unset_list, 1)
catch /E1130:/
  echo 'null list'
endtry
var unset_dict = null_dict
unset_dict.key = 1
echo unset_dict
echo type(null_list) typename(null_list) null_list == [] null_list is []
echo null_list == null [] == null null_list is null
echo null_string == '' null_string is ''
echo type(pair) == v:t_tuple
var [first, second; rest] = [1, 2, 3, 4]
echo first second rest
var text =<< trim END
  literal # text
  another line
END
echo text
{
  var scoped = 'inside'
  echo scoped
}
echo exists('scoped')
def ConstantBranch(): number
  if false
    ThisIsNotACommand
  endif
  return 6
enddef
defcompile ConstantBranch
echo ConstantBranch()
echo 017 0o17
var one = Base.new(3)
var two = Base.new(3)
echo one == two one is two string(one)
