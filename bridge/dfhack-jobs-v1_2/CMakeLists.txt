project(dfmcp_jobs_v1_2)
# Separate, unadmitted jobs-only profile. Does not rebuild either existing bridge.
dfhack_plugin(dfmcp_jobs_v1_2 dfmcp_jobs_v1_2.cpp PROTOBUFS DfmcpJobsV1_2)
