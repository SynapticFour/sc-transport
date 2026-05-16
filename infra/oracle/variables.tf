variable "tenancy_ocid" {
  type = string
}

variable "user_ocid" {
  type = string
}

variable "fingerprint" {
  type = string
}

variable "private_key" {
  type      = string
  sensitive = true
}

variable "compartment_ocid" {
  type = string
}

variable "ssh_public_key" {
  type = string
}

variable "sender_region" {
  type    = string
  default = "eu-frankfurt-1"
}

variable "receiver_region" {
  type    = string
  default = "us-ashburn-1"
}

variable "github_repo" {
  type        = string
  description = "GitHub org/repo cloned on VMs (public repo)."
  default     = "SynapticFour/sc-transport"
}

variable "github_ref" {
  type    = string
  default = "main"
}

variable "sct_port" {
  type        = number
  description = "sct recv listen port on receiver VM."
  default     = 9410
}
