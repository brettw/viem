vim9script
def Identity<T>(x: T): T
  return x
enddef
var Reference = Identity<number>
echo Reference(3)
def Combine<T>(left: T): string
  def Inner<U>(right: U): string
    return string(left) .. '/' .. string(right)
  enddef
  return Inner<number>(5)
enddef
echo Combine<string>('left')

class Box
  var value: any
  def new<T>(value: T)
    this.value = value
  enddef
  def Get<T>(): T
    return this.value
  enddef
  static def Identity<T>(value: T): T
    return value
  enddef
endclass
class Child extends Box
  def new<T>(value: T)
    this.value = value
  enddef
  def Get<U>(): U
    return this.value
  enddef
endclass
def ReadBox(): string
  var box = Box.new<string>('box')
  var child = Child.new<number>(7)
  return box.Get<string>() .. '/' .. string(child.Get<number>())
enddef
echo ReadBox()
echo Box.Identity<number>(9)
def ReadStatic(): bool
  return Box.Identity<bool>(true)
enddef
echo ReadStatic()
var Constructor = Box.new<number>
echo Constructor(11).Get<number>()
var Reader = Constructor(13).Get<number>
echo Reader()

enum Pair
  Text<string>('value'),
  Integer<number>(42)
  const value: any
  def new<T>(value: T)
    this.value = value
  enddef
endenum
echo Pair.Text.value Pair.Integer.value
try
  Box.new(1)
catch
  echo matchstr(v:exception, 'E\d\+')
endtry
try
  Identity<number, string>(1)
catch
  echo matchstr(v:exception, 'E\d\+')
endtry
