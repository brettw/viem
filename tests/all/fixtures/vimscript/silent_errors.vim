" Which silenced errors still skip the rest of their line. Run through
" execute() so Ex mode's :silent quirks do not reach the recording.
let g:r = []
function! Run(line)
  let g:out = execute(a:line)
endfunction
call Run("silent! echo nosuch | call add(g:r, 'after-echo')")
call Run("silent! call Nosuch() | call add(g:r, 'after-call')")
call Run("silent! let x = nosuch | call add(g:r, 'after-let')")
call Run("silent! nosuchcmd | call add(g:r, 'after-cmd')")
call Run("silent! call add(g:r, nosuch) | call add(g:r, 'after-call-arg')")
call Run("try | silent! call Nosuch() | call add(g:r, 'in-try') | catch | call add(g:r, 'caught') | endtry")
echo g:r
echo v:errmsg
function! Fails()
  silent! echo nosuch | call add(g:r, 'in-function')
  return 1
endfunction
call Run("call Fails() | call add(g:r, 'after-function')")
echo g:r[-2:]
