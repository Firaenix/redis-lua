/// Take another unit as the inner of the script.
pub trait TakeScript<I> {
    type Item;

    fn take(self, inner: I) -> Self::Item;
}

/// Generated script information
#[derive(Clone, Debug)]
pub struct Info {
    /// The entire script including arguments initialization
    script: &'static str,
    /// The script excluding arguments initialization
    body: &'static str,
    /// The list of arguments.
    args: &'static [&'static str],
}

impl Info {
    pub fn new(script: &'static str, body: &'static str, args: &'static [&'static str]) -> Self {
        Self { script, body, args }
    }
}

/// A complete invocable script unit.
pub trait Script: Sized {
    fn apply(self, invoke: &mut redis::ScriptInvocation);

    fn info(&self, _: &mut Vec<Info>);

    fn join<T: Script>(self, other: T) -> ScriptJoin<Self, T> {
        ScriptJoin(self, other)
    }

    fn invoke<T>(self, con: &mut dyn redis::ConnectionLike) -> redis::RedisResult<T>
    where
        T: redis::FromRedisValue,
    {
        let mut info = vec![];
        self.info(&mut info);
        let script = gen_script(&info);
        let mut invoke = script.prepare_invoke();
        self.apply(&mut invoke);
        invoke.invoke(con)
    }

    fn invoke_async<C, T>(self, con: C) -> redis::RedisFuture<(C, T)>
    where
        C: redis::aio::ConnectionLike + Clone + Send + 'static,
        T: redis::FromRedisValue + Send + 'static,
    {
        let mut info = vec![];
        self.info(&mut info);
        let script = gen_script(&info);
        let mut invoke = script.prepare_invoke();
        self.apply(&mut invoke);
        Box::new(invoke.invoke_async(con))
    }
}

impl Script for () {
    fn apply(self, _: &mut redis::ScriptInvocation) {}

    fn info(&self, _: &mut Vec<Info>) {}
}

pub struct ScriptJoin<S, T>(S, T);

impl<S, T> Script for ScriptJoin<S, T>
where
    S: Script,
    T: Script,
{
    fn apply(self, invoke: &mut redis::ScriptInvocation) {
        self.0.apply(invoke);
        self.1.apply(invoke);
    }

    fn info(&self, info: &mut Vec<Info>) {
        self.0.info(info);
        self.1.info(info);
    }
}

/// Generate a script from a list of script information
pub fn gen_script(info: &[Info]) -> redis::Script {
    if info.len() == 1 {
        // Single script
        let script = info.get(0).expect("At leasts one script must exist").script;
        redis::Script::new(script)
    } else {
        // Generate the joined script.
        let mut arg_index = 0;
        let mut script = String::new();
        let last = info.len() - 1;
        for (index, info) in info.iter().enumerate() {
            let prefix = if index == last { "return " } else { "" };
            let mut init = String::new();

            for arg in info.args {
                arg_index += 1;
                init += &format!("local {} = ARGV[{}] ", arg, arg_index);
            }

            script += &format!("{}(function() {} {} end)();\n", prefix, init, info.body);
        }
        redis::Script::new(&script)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ```
    // let a = 10;
    // let b = 20;
    // let script = lua! {
    //    return @a + $a + @b + $b
    // };
    //
    // script.a(10).b(30).invoke(&mut con);
    // ```
    //
    // @a ... a1: A1
    // $a ... a2: A2
    // @b ... a3: A3
    // $b ... a4: A4

    // *** Generate script root
    /// Represents `Script`, which is clonable if the arguments are clonable.
    #[derive(Clone, Debug)]
    struct Chain0<I, A1, A3> {
        info: Info,
        inner: I,
        a1: A1,
        a3: A3,
    }

    impl<I, A1, A3> Chain0<I, A1, A3> {
        fn new(info: Info, inner: I, a1: A1, a3: A3) -> Self {
            Self {
                info,
                inner,
                a1,
                a3,
            }
        }

        fn a<A2>(self, var: A2) -> Chain1<I, A1, A3, A2> {
            Chain1 {
                info: self.info,
                inner: self.inner,
                a1: self.a1,
                a3: self.a3,
                a2: var,
            }
        }
    }

    impl<I, I2, A1, A3> TakeScript<I> for Chain0<I2, A1, A3>
    where
        I: Script,
        I2: Script,
    {
        type Item = Chain0<ScriptJoin<I, I2>, A1, A3>;

        fn take(self, inner: I) -> Self::Item {
            Self::Item {
                info: self.info,
                inner: inner.join(self.inner),
                a1: self.a1,
                a3: self.a3,
            }
        }
    }

    impl<I, S, A1, A3> std::ops::Add<S> for Chain0<I, A1, A3> {
        type Output = PartialChain0<I, S, A1, A3>;

        fn add(self, other: S) -> Self::Output {
            PartialChain0::new(self, other)
        }
    }

    // If the script doesn't have variable, this code will be generated instead.
    //
    // impl<I, S, A1, A3> std::ops::Add<S> for Chain0<I, A1, A3>
    // where
    //     S: TakeScript<Chain0<I, A1, A3>>,
    // {
    //     type Output = S::Item;
    //
    //     fn add(self, other: S) -> Self::Output {
    //         other.take(self)
    //     }
    // }

    // *** Generate a chain piece (repeat)
    #[derive(Clone, Debug)]
    struct Chain1<I, A1, A3, A2> {
        info: Info,
        inner: I,
        a1: A1,
        a3: A3,
        a2: A2,
    }

    impl<I, A1, A3, A2> Chain1<I, A1, A3, A2> {
        fn b<A4>(self, var: A4) -> Chain2<I, A1, A3, A2, A4> {
            Chain2 {
                info: self.info,
                inner: self.inner,
                a1: self.a1,
                a3: self.a3,
                a2: self.a2,
                a4: var,
            }
        }
    }

    // *** Generate the last chain piece (invoke and impl unit)
    #[derive(Clone, Debug)]
    struct Chain2<I, A1, A3, A2, A4> {
        info: Info,
        inner: I,
        a1: A1,
        a3: A3,
        a2: A2,
        a4: A4,
    }

    impl<I, A1, A3, A2, A4> Chain2<I, A1, A3, A2, A4>
    where
        I: Script,
        A1: redis::ToRedisArgs,
        A3: redis::ToRedisArgs,
        A2: redis::ToRedisArgs,
        A4: redis::ToRedisArgs,
    {
        fn invoke<T>(self, con: &mut dyn redis::ConnectionLike) -> redis::RedisResult<T>
        where
            T: redis::FromRedisValue,
        {
            let mut info = vec![];
            self.info(&mut info);
            let script = gen_script(&info);
            let mut invoke = script.prepare_invoke();
            self.apply(&mut invoke);
            invoke.invoke(con)
        }
    }

    impl<I, A1, A3, A2, A4> Script for Chain2<I, A1, A3, A2, A4>
    where
        I: Script,
        A1: redis::ToRedisArgs,
        A3: redis::ToRedisArgs,
        A2: redis::ToRedisArgs,
        A4: redis::ToRedisArgs,
    {
        fn apply(self, invoke: &mut redis::ScriptInvocation) {
            self.inner.apply(invoke);
            invoke.arg(self.a1);
            invoke.arg(self.a2);
            invoke.arg(self.a3);
            invoke.arg(self.a4);
        }

        fn info(&self, info: &mut Vec<Info>) {
            self.inner.info(info);
            info.push(self.info.clone());
        }
    }

    // *** Generate the partial script root
    #[derive(Clone, Debug)]
    struct PartialChain0<I, S, A1, A3> {
        script: Chain0<I, A1, A3>,
        next: S,
    }

    impl<I, S, A1, A3> PartialChain0<I, S, A1, A3> {
        fn new(script: Chain0<I, A1, A3>, next: S) -> Self {
            Self { script, next }
        }

        fn a<A2>(self, var: A2) -> PartialChain1<I, S, A1, A3, A2> {
            PartialChain1 {
                chain: self.script.a(var),
                next: self.next,
            }
        }
    }

    impl<I, I2, S, A1, A3> TakeScript<I> for PartialChain0<I2, S, A1, A3>
    where
        I: Script,
        I2: Script,
    {
        type Item = PartialChain0<ScriptJoin<I, I2>, S, A1, A3>;

        fn take(self, inner: I) -> Self::Item {
            Self::Item {
                script: Chain0 {
                    info: self.script.info,
                    inner: inner.join(self.script.inner),
                    a1: self.script.a1,
                    a3: self.script.a3,
                },
                next: self.next,
            }
        }
    }

    impl<I, S2, S1, A1, A3> std::ops::Add<S2> for PartialChain0<I, S1, A1, A3>
    where
        S1: std::ops::Add<S2>,
    {
        type Output = PartialChain0<I, S1::Output, A1, A3>;

        fn add(self, other: S2) -> Self::Output {
            PartialChain0::new(self.script, self.next + other)
        }
    }

    // *** Generate a chain piece of partial script (repeat)
    // #[derive(Clone, Debug)]
    // struct PartialChain1<I, S, A1, A3, A2> {
    //     chain: Chain1<I, A1, A3, A2>,
    //     next: S,
    // }
    //
    // impl<I, S, A1, A3, A2> PartialChain1<I, S, A1, A3, A2>
    // {
    //     fn b<A4>(self, var: A4) -> PartialChain2<I, S, A1, A3, A2, A4>
    //     {
    //         PartialChain2 {
    //             chain: self.chain.b(var),
    //             next: self.next,
    //         }
    //     }
    // }

    // *** Generate the last piece of the partial script
    #[derive(Clone, Debug)]
    struct PartialChain1<I, S, A1, A3, A2> {
        chain: Chain1<I, A1, A3, A2>,
        next: S,
    }

    impl<I, S, A1, A3, A2> PartialChain1<I, S, A1, A3, A2>
    where
        A1: redis::ToRedisArgs + 'static,
        A3: redis::ToRedisArgs + 'static,
        A2: redis::ToRedisArgs + 'static,
    {
        fn b<A4>(self, var: A4) -> S::Item
        where
            S: TakeScript<Chain2<I, A1, A3, A2, A4>>,
            A4: redis::ToRedisArgs + 'static,
        {
            let chain = self.chain.b(var);
            let next = self.next;
            next.take(chain)
        }
    }

    #[test]
    fn generated() {
        let x = 10;
        let y = -2;

        let script = Chain0::new(
            Info::new(
                r#"
local _a1 = ARGV[1];
local _a2 = ARGV[2];
local _a3 = ARGV[3];
local _a4 = ARGV[4];
return _a1 - _a2 - _a3 + _a4;
"#,
                r#"
return _a1 - _a2 - _a3 + _a4;
"#,
                &["_a1", "_a2", "_a3", "_b4"],
            ),
            (),
            x,
            y,
        );
        let script2 = script.clone();

        let cli = redis::Client::open("redis://127.0.0.1").unwrap();
        let mut con = cli.get_connection().unwrap();
        let ret: isize = script.a(10).b(3).invoke(&mut con).unwrap();
        assert_eq!(ret, 5);
        let ret: isize = script2.a(11).b(-4).invoke(&mut con).unwrap();
        assert_eq!(ret, -3);
    }

    #[test]
    fn generated_join() {
        let x = 10;
        let y = -2;

        let script = Chain0::new(
            Info::new(
                r#"
local _a1 = ARGV[1];
local _a2 = ARGV[2];
local _a3 = ARGV[3];
local _a4 = ARGV[4];
return _a1 - _a2 - _a3 + _a4;
"#,
                r#"
return _a1 - _a2 - _a3 + _a4;
"#,
                &["_a1", "_a2", "_a3", "_a4"],
            ),
            (),
            x,
            y,
        );
        let script2 = script.clone();

        let scriptj = script + script2;

        let cli = redis::Client::open("redis://127.0.0.1").unwrap();
        let mut con = cli.get_connection().unwrap();
        let ret: isize = scriptj.a(10).b(3).a(11).b(-4).invoke(&mut con).unwrap();
        assert_eq!(ret, -3);
    }

    #[test]
    fn generated_join3() {
        let x = 10;
        let y = -2;

        let script = Chain0::new(
            Info::new(
                r#"
local _a1 = ARGV[1];
local _a2 = ARGV[2];
local _a3 = ARGV[3];
local _a4 = ARGV[4];
return _a1 - _a2 - _a3 + _a4;
"#,
                r#"
return _a1 - _a2 - _a3 + _a4;
"#,
                &["_a1", "_a2", "_a3", "_a4"],
            ),
            (),
            x,
            y,
        );
        let script2 = script.clone();
        let script3 = script.clone();

        let scriptj = script + script2 + script3;

        let cli = redis::Client::open("redis://127.0.0.1").unwrap();
        let mut con = cli.get_connection().unwrap();
        let ret: isize = scriptj
            .a(10)
            .b(3)
            .a(3)
            .b(9)
            .a(11)
            .b(-4)
            .invoke(&mut con)
            .unwrap();
        assert_eq!(ret, -3);
    }
}
