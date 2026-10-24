fun add(a: i32, b: i32): i32 {
  return a + b;
}

fun sub(a: i32, b: i32): i32 {
  return a - b;
}

fun mul(a: i32, b: i32): i32 {
  return a * b;
}

fun quo(a: i32, b: i32): i32 {
  return a / b;
}

fun rem(a: i32, b: i32): i32 {
  return a % b;
}

fun abs(n: i32): i32 {
  if n < 0 {
    return -n;
  }
  return n;
}

fun fib(n: i32): i32 {
  if n < 2 {
    return n;
  }
  return fib(n - 1) + fib(n - 2);
}

fun sumto(n: i32): i32 {
  var i: i32 = 1;
  var s: i32 = 0;
  while i <= n {
    s = s + i;
    i = i + 1;
  }
  return s;
}

fun dist(a: i32, b: i32): i32 {
  return abs(sub(a, b));
}
