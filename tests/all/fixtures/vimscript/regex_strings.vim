" Vim patterns in expressions.
echo 'foobar' =~ 'o\+b' 'foobar' =~ '^bar' 'foobar' !~ 'x'
echo 'FooBar' =~ '\cfoobar' 'FooBar' =~ '\Cfoobar'
echo 'abc123' =~ '\d\+$' 'x y' =~ '\s' 'word' =~ '\<word\>'
echo matchstr('key=value', '=\zs.*') matchstr('key=value', '.*\ze=')
echo substitute('a,b,c', ',', ';', 'g') substitute('2024-01-15', '\v(\d+)-(\d+)-(\d+)', '\3/\2/\1', '')
echo substitute('hello', '\(.\)', '\u\1', '') substitute('HELLO world', '\w\+', '\L&', 'g')
echo split('one  two three', '\s\+') split('a1b2c3', '\d')
echo matchlist('2024-01-15', '\(\d\+\)-\(\d\+\)')[1:2]
echo 'path/to/file.txt' =~# '\.txt$' tolower('ABC') =~ '^abc$'
echo substitute('a.b.c', '\.', '-', 'g') substitute('x', 'x', '\n', '') =~ "\n"
