output "sender_ip" {
  description = "Public IP of the WAN test sender VM."
  value       = data.oci_core_vnic.sender_vnic.public_ip_address
}

output "receiver_ip" {
  description = "Public IP of the WAN test receiver VM."
  value       = data.oci_core_vnic.receiver_vnic.public_ip_address
}

output "sender_region" {
  value = var.sender_region
}

output "receiver_region" {
  value = var.receiver_region
}

output "sct_port" {
  value = var.sct_port
}
