use viem_core::document::{Document, Encoding, Format};
use pulldown_cmark::{Parser,Options};
fn main(){
 if std::env::args().any(|a| a=="--fuzz") { return fuzz(); }
 let cases = [
  "|a|\r|-|\na\r# b", "|a|\r|-|\na", "|a|\r|-|\na\r", "|a|\r|-|", "a\r-\na\r", "|a|\r|-|\nx\r# y",
  "a\r# b", "a\r  # b", "a\r---", "a\n\u{a0}\nb", "a\n\u{3000}\nb", "a\n\u{2003}\nb",
  "a\n  # b", "a\n # b", "a\n   # b", "a\n# b", "a\n #\nb", "a\n  ---\nb", "a\n  ***\nb", "> a\n>   # b", "- a\n  <div>b</div>", "- a\n  |x|\n  |-|",
  ">\t\tfoo", ">\t    foo", "-\t\tfoo", "-\t    foo", "- a\n  ```\n  \tfoo\n  ```",
  "foo\\\\\nbar", "foo\\\\\\\nbar", "foo\t\nbar", "foo  \t\nbar",
  "> a\n    b", "> a\n>     b", "> a\n>\n>\tb", ">\tfoo\n>\tbar", "-\tfoo\n\tbar",
  "-     code\n      next", "- ```\n  code\n  ```", "- ```\n  code\n```\nafter", "- ```\n  code\n  ```\nafter",
  "a\n-\nb", "a\n*\nb", "a\n+\nb", "a\n1.\nb", "a\n2.\nb", "- a\n-\n- b",
  "- a\n  # heading\n  prose", "- a\n  ---\n  prose", "- a\n\n  # heading\n  prose", "- a\n\n  > quote\n  tail",
  "#\n##\n###", "> #\n> ##\n> ###", "- #\n- ##\n- ###",
  "> a\n> -\n> b", "> a\n-\nb", "- > a\n  > b", "- > a\n  b", "- a\n\n\tb",
  "|a|b|\n|-|-|\n| x | y |", "- |a|b|\n  |-|-|\n  |x|y|", "- > |a|b|\n  > |-|-|\n  > |x|y|",
 ];
 for source in cases { 
  println!("SRC {source:?}");
  println!("PARSE {:?}",Parser::new_ext(source,Options::ENABLE_TABLES).collect::<Vec<_>>());
  for format in [Format::Markdown,Format::MarkdownSource] {
   let d=std::panic::catch_unwind(|| Document::from_bytes(source.as_bytes().to_vec(),Encoding::Utf8,format));
   match d { Ok(Ok(d))=>println!("VIEM {:?} {:?} {:?}",format,d.text(),d.projection().blocks().iter().map(|b|(&b.range,&b.kind,&b.style,b.quote_depth)).collect::<Vec<_>>()),Ok(Err(e))=>println!("FAIL {e:?}"),Err(_)=>println!("PANIC") }
  }
 }
}

fn fuzz() {
 std::panic::set_hook(Box::new(|_|{}));
 let atoms=["a\r# b","|a|\r|-|",">\t\tfoo","\u{a0}","é\r# b","a\r","\u{0}","a","# b","##","-","*","1.","2.","- a","  # b","    a",">"," > a","> >","```","```a"," ```","~~~","    ```","<div>","<span>","<pre>","</div>","</pre>","<!--","-->","é","ü","|a|","|-|","","  ","\\","  > # b","- > a","- \t\tcode","<img src='x'>"];
 let mut seed=91873u64;let mut failures=0;
 for case in 0..10000 {let mut source=String::new(); for _ in 0..(case%5+2) {seed=seed.wrapping_mul(6364136223846793005).wrapping_add(1);if !source.is_empty(){source.push('\n');}source.push_str(atoms[(seed as usize>>7)%atoms.len()]);} 
  for format in [Format::Markdown,Format::MarkdownSource] {let out=std::panic::catch_unwind(||Document::from_bytes(source.as_bytes().to_vec(),Encoding::Utf8,format)); match out {Err(_)=>{println!("PANIC {format:?} {source:?}");failures+=1},Ok(Err(e))=>{println!("ERR {format:?} {source:?} {e:?}");failures+=1},_=>{}}}
 }println!("DONE failures={failures}");
}
