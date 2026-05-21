use crate::{
    cli::DoctorArgs,
    error::{Error, Result},
    inspection,
    output::{self, OutputFormat},
    workspace::Workspace,
};

pub fn doctor_command(workspace: &Workspace, args: DoctorArgs) -> Result<()> {
    let report = inspection::run_doctor(workspace, args.fix)?;
    let errors = report.error_count();
    let format = OutputFormat::from_json_flag(args.output.json);
    print!("{}", output::render_doctor_report(format, &report)?);
    if errors > 0 {
        Err(Error::DoctorIssues(errors))
    } else {
        Ok(())
    }
}
