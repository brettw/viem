" :try, :catch, :finally and :throw.
try
  throw 'custom'
catch /^cust/
  echo 'caught' v:exception
endtry
try
  call NoSuchFunction()
catch /E117/
  echo 'caught' v:exception
endtry
try
  let x = undefined_var
catch /E121/
  echo 'caught' v:exception
endtry
try
  try
    throw 'inner'
  finally
    echo 'inner finally'
  endtry
catch
  echo 'outer caught' v:exception
endtry
try
  try
    throw 'x'
  catch
    echo 'in catch'
    call Undefined()
  endtry
catch
  echo 'error in catch:' v:exception
endtry
try
  try
    echo 'body'
  finally
    call Undefined2()
  endtry
catch
  echo 'error in finally:' v:exception
endtry
function! Thrower()
  throw 'from function'
endfunction
try
  call Thrower()
  echo 'not reached'
catch
  echo 'function threw:' v:exception
endtry
function! ReturnInFinally()
  try
    return 'body'
  finally
    echo 'finally runs'
  endtry
endfunction
echo ReturnInFinally()
try
  throw 'first'
catch /second/
  echo 'wrong'
catch /first/
  echo 'second catch' v:exception
finally
  echo 'finally after catch'
endtry
echo 'exception after' v:exception '|'
try
  throw 'Vim:fake'
catch
  echo v:exception
endtry
try
  echoerr 'from echoerr'
catch
  echo v:exception
endtry
try
  throw ['a', 'list']
catch
  echo string(v:exception)
endtry
