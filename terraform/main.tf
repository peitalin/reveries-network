terraform {
  required_providers {
    google-beta = {
      source  = "hashicorp/google-beta"
      version = "~> 6.21"
    }
  }
}

variable "project_id" {
  description = "GCP Project ID"
  type        = string
  default     = "eigen-413918"
}

variable "zone" {
  description = "GCP Zone"
  type        = string
  default     = "asia-southeast1-a"
}

### Multizone deployment
# variable "zones" {
#   description = "GCP Zones for deployment"
#   type        = list(string)
#   default     = [
#     "asia-southeast1-a",
#     "us-central1-a",
#     "europe-west4-a"
#   ]
# }

variable "github_token" {
  description = "GitHub Personal Access Token"
  type        = string
  sensitive   = true
}

variable "github_repo" {
  description = "GitHub repository URL (format: owner/repo)"
  type        = string
}

variable "repo_branch" {
  description = "Repository branch to clone"
  type        = string
  default     = "develop"
}

variable "service_account_email" {
  description = "GCP Service Account Email"
  type        = string
}

variable "home_dir" {
  description = "user's home directory"
  type        = string
  default     = "/home/n6378056"
}

locals {
  timestamp = formatdate("YYYYMMDD-hhmmss", timestamp())
  node_commands = ["node1-prod", "node2-prod", "node3-prod", "node4-prod", "node5-prod"]

  ### Multizone deployment: Get regions from zones
  # regions = [for zone in var.zones : substr(zone, 0, length(zone)-2)]

  startup_script = <<-EOF
    #!/bin/bash

    # Basic logging to syslog
    exec 1> >(logger -s -t $(basename $0)) 2>&1

    cd ~

    # Install system dependencies
    apt-get update && apt-get install -y \
      build-essential \
      curl \
      git \
      pkg-config \
      libssl-dev \
      libtss2-dev \
      rustup

    # Set up Rust
    rustup default stable

    # Install Just
    sudo snap install just --classic

    # Clone the repository
    sudo git clone https://${var.github_token}@github.com/${var.github_repo}.git ${var.home_dir}/1up-network
    cd ${var.home_dir}/1up-network
    git checkout ${var.repo_branch}

    # # Run the justfile command for this node
    # just %NODE_COMMAND%
  EOF
}

# Must use google-beta provider to create TDX instances
# confidential_instance_type is not supported in the default google provider
# https://github.com/hashicorp/terraform-provider-google-beta?tab=readme-ov-file
provider "google-beta" {
  project = var.project_id
  zone    = var.zone
  region = substr(var.zone, 0, length(var.zone)-2)
}

# Static IP address for node1
resource "google_compute_address" "node1_static_ip" {
  provider     = google-beta
  name         = "tee-node1-static-ip"
  region       = substr(var.zone, 0, length(var.zone)-2)
  address_type = "EXTERNAL"
}

# Static IP address for node2
resource "google_compute_address" "node2_static_ip" {
  provider     = google-beta
  name         = "tee-node2-static-ip"
  region       = substr(var.zone, 0, length(var.zone)-2)
  address_type = "EXTERNAL"
}

# Add this firewall rule for RPC port
resource "google_compute_firewall" "allow_rpc_ports" {
  provider = google-beta
  name     = "tee-node-${local.timestamp}-allow-rpc-ports"
  network  = "projects/${var.project_id}/global/networks/default"
  project  = var.project_id

  allow {
    protocol = "tcp"
    ports    = ["22", "80", "8000-9999"]  # port 22 for SSH
  }

  allow {
    protocol = "udp"
    ports    = ["22", "80", "8000-9999"]
  }

  source_ranges = ["0.0.0.0/0"]
  target_tags   = ["allow-rpc-ports"]
}

# GCP Confidential VM setup
# Instance Type: c3-standard-* family
# Operating System: containerOS, RHEL0, SLES-15-sp5, Ubuntu 22.04
# Supported Zones: asia-southeast-1-{a,b,c}, europe-west4-{a,b}, us-central1-{a,b,c}
#
# For more info on supported operating systems, check out GCP supported configurations:
# https://cloud.google.com/confidential-computing/confidential-vm/docs/supported-configurations#intel-tdx
# Currently, TDX enabled VMs can only be created via gcloud or Rest API:
# https://cloud.google.com/confidential-computing/confidential-vm/docs/create-a-confidential-vm-instance#gcloud

resource "google_compute_instance" "tdx_instances" {
  provider = google-beta
  count    = 3
  name     = "tee-node${count.index + 1}-${local.timestamp}"
  machine_type = "c3-standard-4"
  zone         = var.zone
  ### Multizone deployment:
  # zone        = var.zones[count.index]


  // Enable Confidential Computing
  confidential_instance_config {
    enable_confidential_compute = true
    confidential_instance_type  = "TDX"
  }

  boot_disk {
    auto_delete = true
    device_name = "boot-disk-tee-node${count.index + 1}-${local.timestamp}"
    initialize_params {
      image = "projects/ubuntu-os-cloud/global/images/ubuntu-2404-noble-amd64-v20250214"
      size  = 20
      type  = "hyperdisk-balanced"
    }
  }

  network_interface {
    network = "default"
    access_config {
      # Use static IPs for node1 and node2, ephemeral IPs for nodes 3-5
      nat_ip = count.index == 0 ? google_compute_address.node1_static_ip.address : (
               count.index == 1 ? google_compute_address.node2_static_ip.address : null)
    }
  }

  service_account {
    email  = var.service_account_email
    scopes = ["cloud-platform"]
  }

  scheduling {
    on_host_maintenance = "TERMINATE"
  }

  shielded_instance_config {
    enable_secure_boot = false
    enable_vtpm = true
    enable_integrity_monitoring = true
  }

  metadata = {
    startup-script = replace(local.startup_script, "%NODE_COMMAND%", local.node_commands[count.index])
  }

  tags = ["http-server", "https-server", "allow-rpc-ports"]

  labels = {
    goog-ec-src = "vm_add-gcloud"
  }

  reservation_affinity {
    type = "ANY_RESERVATION"
  }

  deletion_protection = false
}

### Multizone deployment:
# Output variables for monitoring
# output "instance_details" {
#   description = "Details for all instances"
#   value = [
#     for i in range(3) : {
#       name = google_compute_instance.tdx_instances[i].name
#       zone = var.zones[i]
#       ip   = google_compute_address.static_ips[i].address
#       monitoring_command = "gcloud compute instances tail-serial-port-output ${google_compute_instance.tdx_instances[i].name} --zone=${var.zones[i]}"
#     }
#   ]
# }

# output "monitoring_commands" {
#   description = "Commands to monitor startup script output for each instance"
#   value = [
#     for i in range(3) : "gcloud compute instances tail-serial-port-output ${google_compute_instance.tdx_instances[i].name} --zone=${var.zones[i]}"
#   ]
# }

output "instance_names" {
  description = "The names of the instances"
  value       = google_compute_instance.tdx_instances[*].name
}

output "instance_ips" {
  description = "The external IPs of the instances"
  value = [
    for i, instance in google_compute_instance.tdx_instances : {
      node     = "node${i + 1}"
      ip_type  = i < 2 ? "static" : "ephemeral"
      ip       = i == 0 ? google_compute_address.node1_static_ip.address : (
                 i == 1 ? google_compute_address.node2_static_ip.address :
                 instance.network_interface[0].access_config[0].nat_ip)
    }
  ]
}

output "instance_zone" {
  description = "The zone where instances are deployed"
  value       = var.zone
}

output "monitoring_commands" {
  description = "Commands to monitor startup script output for each instance"
  value       = [for instance in google_compute_instance.tdx_instances : "gcloud compute instances tail-serial-port-output ${instance.name} --zone=${var.zone}"]
}