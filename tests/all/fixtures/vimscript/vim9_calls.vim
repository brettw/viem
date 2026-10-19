vim9script
var Lambda = (x: string, ...ys: list<string>): string => x .. join(ys, '')
echo Lambda(')') Lambda('a', 'b', 'c')
var Func: func: number = () => 7
echo Func()
echo 'abc'[-4] 'abc'[-5 : 1]
def PartialReturn(flag: bool): number
  if flag
    return 1
  endif
enddef
try
  defcompile PartialReturn
catch
  echo v:exception
endtry
def BothReturn(flag: bool): number
  if flag
    return 1
  else
    return 2
  endif
enddef
defcompile BothReturn
echo BothReturn(false)
class Counter
  public var count: number = 0
  def Add(n: number)
    this.count += n
  enddef
endclass
var counter = Counter.new()
var AddTwo = function(counter.Add, [2])
AddTwo()
def DoDeferred(c: Counter)
  defer c.Add(3)
enddef
DoDeferred(counter)
echo counter.count
echo true ? (1) : 2
echo false ? (1) : (2)
