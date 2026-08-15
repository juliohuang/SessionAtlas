[CmdletBinding()]
param(
    [string[]]$Projects = @(
        "SessionAtlas.csproj",
        "SessionAtlas.Tests/SessionAtlas.Tests.csproj",
        "SessionAtlas.Desktop/SessionAtlas.Desktop.csproj",
        "SessionAtlas.Desktop.Tests/SessionAtlas.Desktop.Tests.csproj"
    )
)

$ErrorActionPreference = "Stop"
$findings = [System.Collections.Generic.List[string]]::new()

foreach ($project in $Projects) {
    $jsonText = & dotnet list $project package --vulnerable --include-transitive --format json
    if ($LASTEXITCODE -ne 0) {
        throw "dotnet list failed for $project with exit code $LASTEXITCODE."
    }
    $report = $jsonText | ConvertFrom-Json
    foreach ($projectReport in @($report.projects)) {
        $frameworksProperty = $projectReport.PSObject.Properties["frameworks"]
        if ($null -eq $frameworksProperty) { continue }
        foreach ($framework in @($frameworksProperty.Value)) {
            if ($null -eq $framework) { continue }
            foreach ($collectionName in @("topLevelPackages", "transitivePackages")) {
                $property = $framework.PSObject.Properties[$collectionName]
                if ($null -eq $property) { continue }
                foreach ($package in @($property.Value)) {
                    if (@($package.vulnerabilities).Count -eq 0) { continue }
                    foreach ($vulnerability in @($package.vulnerabilities)) {
                        $findings.Add(
                            "$($projectReport.path): $($package.id) $($package.resolvedVersion) " +
                            "$($vulnerability.severity) $($vulnerability.advisoryurl)")
                    }
                }
            }
        }
    }
}

if ($findings.Count -ne 0) {
    $findings | ForEach-Object { Write-Error $_ }
    throw "NuGet vulnerability audit found $($findings.Count) advisory match(es)."
}

Write-Output "NuGet vulnerability audit passed for $($Projects.Count) projects."
