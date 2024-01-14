use nom::{bytes::complete::tag, multi::separated_list0, IResult};

type Thing = Vec<u8>;

pub struct Rule {
    targets: Vec<Thing>,
    prequisites: Vec<Thing>,
    commands: Vec<Thing>,
}

fn path(input: &str) -> IResult<&str, Thing> {
    todo!()
}

fn rule(input: &str) -> IResult<&str, Rule> {
    let (input, targets) = separated_list0(tag(" "), path)(input)?;
    let rule = Rule {
        targets,
        prequisites: todo!(),
        commands: todo!(),
    };
    Ok((input, rule))
}
