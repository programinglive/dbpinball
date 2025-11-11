use anyhow::Result;

const SERVICE_NAME: &str = "dbpinball";

pub fn store_password(id: &str, password: &str) -> Result<()> {
    let entry = keyring::Entry::new(SERVICE_NAME, id)?;
    entry.set_password(password)?;
    Ok(())
}

pub fn get_password(id: &str) -> Result<String> {
    let entry = keyring::Entry::new(SERVICE_NAME, id)?;
    let password = entry.get_password()?;
    Ok(password)
}

pub fn delete_password(id: &str) -> Result<()> {
    let entry = keyring::Entry::new(SERVICE_NAME, id)?;
    entry.delete_password()?;
    Ok(())
}
