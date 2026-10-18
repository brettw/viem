" Numbers, floats, strings and operators.
echo 1 + 2 * 3 (1 + 2) * 3 7 / 2 -7 / 2 7 % 3 -7 % 3
echo 5 / 0 -5 / 0 0 / 0 5 % 0
echo 9223372036854775807 + 1
echo 0x1F 0b101 017 0o17 019
echo "010" + 0 "0x10" + 0 "3abc" + 1 "abc" + 1 " 5" + 0 "-12" * 2
echo 1.0 / 3 1.0e20 0.000001 123456789.0 1500.0 -0.0 1.5e3 2.5e-2
echo 1.0 / 0 string(-1.0 / 0) string(0.0 / 0)
echo 3 . 4
echo 1 .. 2.5
echo "a" .. 1.5
echo 'it''s' "tab\there" "a\"b" "\x41\u00e9\101"
echo "abc"[1] "abc"[-1] "abc"[1:] "abc"[:1] "abc"[-2:] "abcdef"[1:-2] "abc"[5:]
echo [1, 2, 3][-1] [1, 2, 3][1:] [1, 2, 3][:-2] [1, 2, 3][5:] [1, 2, 3][-5:]
echo 1 == "1" "a" == 0 0 == "" [0] == ["x"] 4 is '4' 0 is []
echo "abc" == "ABC" "abc" ==? "ABC" "abc" ==# "abc" "B" < "a" "B" <? "a"
set ignorecase
echo "abc" == "ABC" "abc" =~ "B" "abc" =~# "B"
set noignorecase
echo "10" < "9" 10 < "9" 1.0 == 1 2.0 is 2.0
echo [1, 2] == [1, 2] [1, 2] is [1, 2] {'a': 1} == {'a': 1}
echo v:true v:false v:null v:none v:true == 1 v:null == 0
echo 1 ? "yes" : "no" 0 ? "yes" : "no" "" ?? "default" "x" ?? "default"
echo !0 !1 !"" -"3" +"4" --5
echo 1 && 0 1 || 0 0 || "" 2 && 3
echo 1 << 3 256 >> 4
echo 0z0102 string(0z0102FF) len(0z010203)
echo $"x={1 + 1} {'s'} {{literal}}" $'a{"b"}c'
echo type(1) type("") type(function("tr")) type([]) type({}) type(1.0) type(v:true) type(v:none) type(0z00)
echo string([1, 'a', [2], {'k': 'v'}]) string(function('strlen')) string(function('tr', [1]))
echo [1, 'a'] {'k': [1]} function('strlen')
echo {-> 42}() {x, y -> x * y}(6, 7) [1, 2, 3]->map({_, v -> v * 2})->join('-')
echo "hello"->toupper() [3, 1, 2]->sort() 'x'->repeat(3)
let s:d = {'n': 1, 'f': {-> 'called'}}
echo s:d.n s:d['n'] s:d.f()
echo #{one: 1, two-x: 2}['two-x']
