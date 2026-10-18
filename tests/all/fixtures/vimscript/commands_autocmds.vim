" User commands, autocommands and :doautocmd.
command! Hello echo 'hello'
Hello
command! -nargs=1 Say echo <q-args>
Say one two
command! -nargs=* Args echo [<f-args>]
Args a b\ c d
command! -nargs=? -bang Bang echo 'bang:<bang>' <q-args>
Bang
Bang! x
command! -nargs=+ -bar Bar echo <q-args> | echo 'after bar'
Bar first
command! -count=3 Count echo <count>
Count
5Count
command Hello2 echo 'two'
Hel
Hello2
command! -nargs=0 NoArgs echo 'none'
NoArgs extra
echo exists(':Hello') exists(':Hel') exists(':H') exists(':Nope')
delcommand Hello
echo exists(':Hello')
augroup Test
  autocmd!
  autocmd User MyEvent echo 'user event fired'
  autocmd User Other* echo 'pattern' expand('<amatch>')
augroup END
doautocmd User MyEvent
doautocmd User OtherThing
autocmd User Once ++once echo 'only once'
doautocmd User Once
doautocmd User Once
echo exists('#Test') exists('#Test#User') exists('#User#MyEvent') exists('#Nope')
augroup Test
  autocmd!
augroup END
doautocmd User MyEvent
echo 'end'
