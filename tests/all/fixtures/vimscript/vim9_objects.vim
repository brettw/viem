vim9script
class Text
  var value: string
  def string(): string
    return '<' .. this.value .. '>'
  enddef
  def empty(): bool
    return this.value == ''
  enddef
  def len(): number
    return strlen(this.value)
  enddef
endclass
class NamedText extends Text
endclass
var text = Text.new('hello')
echo text string(text) len(text) empty(text)
echo [text, NamedText.new('child')]
echo {label: text}
echo empty(Text.new())
def Describe(value: Text): string
  return string(value) .. '/' .. string(len(value))
enddef
echo Describe(text)
class Default
  var n: number = 2
endclass
echo Default.new() empty(Default.new())
try
  len(Default.new())
catch
  echo matchstr(v:exception, 'E\d\+')
endtry
var Length: func(): number = text.len
echo Length()
try
  var Wrong: func(): string = text.len
catch
  echo matchstr(v:exception, 'E\d\+')
endtry
