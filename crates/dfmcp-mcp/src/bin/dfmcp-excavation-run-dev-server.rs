#![forbid(unsafe_code)]
//! Operator-selected, explicitly unadmitted development executable.
use dfmcp_adapter::excavation_run::{ExcavationRegion, FortressIdentity};
use dfmcp_core::{DfmcpError, ErrorCode, Result};
use dfmcp_mcp::live_excavation_run_server::{Configuration, run_stdio};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn invalid() -> DfmcpError { DfmcpError::new(ErrorCode::InvalidRequest,"invalid excavation server operator arguments; use --help") }
fn parse(args: Vec<String>) -> Result<Option<Configuration>> {
    if args==["--help"] {
        println!("dfmcp-excavation-run-dev-server --directory /private/runs --world-folder region1 --site 1 --region '[15,15,2,2,2]' [--initialize]\nRequires DFMCP_ALLOW_UNADMITTED_EXCAVATION_RUN_V1_18=1. Offline sessions need no token. Native control also requires the matching token/endpoint and DFMCP_EXCAVATION_RUN_ALLOW_CLOCK=1. --initialize permits ONE exclusive journal creation, never repair. No designation, global clock fencing, checkpoint, restore or production admission.");
        return Ok(None);
    }
    if args.len()>9||args.iter().any(|v|v.len()>4096||v.contains('\0')){return Err(invalid());}
    let mut pairs=BTreeMap::new();
    let mut initialize=false;
    let mut arguments=args.into_iter();
    while let Some(name)=arguments.next(){
        if name=="--initialize" {
            if initialize{return Err(invalid());}
            initialize=true;
        }else{
            if !["--directory","--world-folder","--site","--region"].contains(&name.as_str()) {return Err(invalid());}
            let value=arguments.next().ok_or_else(invalid)?;
            if pairs.insert(name,value).is_some(){return Err(invalid());}
        }
    }
    let directory=pairs.remove("--directory").ok_or_else(invalid)?;
    let folder=pairs.remove("--world-folder").ok_or_else(invalid)?;
    let site=pairs.remove("--site").ok_or_else(invalid)?;
    let region=pairs.remove("--region").ok_or_else(invalid)?;
    if folder.len()>512||site.len()>10||region.len()>128{return Err(invalid());}
    let number:u32=site.parse().map_err(|_|invalid())?;
    if number.to_string()!=site{return Err(invalid());}
    let region: [u32;5]=serde_json::from_str(&region).map_err(|_|invalid())?;
    Ok(Some(Configuration::new(PathBuf::from(directory),FortressIdentity::new(&folder,number)?,
        ExcavationRegion::new(region)?,initialize)?))
}
fn main() {
    let result=(||{
        let args=std::env::args_os().skip(1).take(10)
            .map(|value|value.into_string().map_err(|_|invalid())).collect::<Result<Vec<_>>>()?;
        if let Some(config)=parse(args)?{run_stdio(config)?;}
        Ok::<(),DfmcpError>(())
    })();
    if let Err(cause)=result{eprintln!("Excavation-run startup refused: {cause}");std::process::exit(2);}
}
#[cfg(test)]
mod tests {
    use super::*;
    fn args()->Vec<String>{["--directory","/private/runs","--world-folder","region1","--site","1","--region","[15,15,2,2,2]"].map(str::to_owned).to_vec()}
    #[test]
    fn operator_configuration_is_bounded_and_not_an_mcp_argument(){
        assert!(parse(args()).is_ok());
        let mut create=args();create.push("--initialize".into());assert!(parse(create).is_ok());
        for (index,value) in [(1,"relative/path"),(5,"01"),(7,"[15,15,2,9,1]"),(0,"--token")]{
            let mut bad=args();bad[index]=value.into();assert!(parse(bad).is_err());
        }
        let mut duplicate=args();duplicate.extend(["--site".into(),"2".into()]);assert!(parse(duplicate).is_err());
    }
}
