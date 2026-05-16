terraform {
  required_version = ">= 1.5.0"

  required_providers {
    oci = {
      source  = "oracle/oci"
      version = "~> 6.0"
    }
  }

  # Configure remote state via `terraform init` (e.g. Terraform Cloud HTTP backend).
  # GitHub Actions uses `terraform init -backend=false` for ephemeral local state.
  backend "http" {}
}
