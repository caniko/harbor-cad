// Thin reference over the pinned OpenLB well-balanced Cahn-Hilliard example.
// https://gitlab.com/openlb/release/-/blob/145cd54810b468f4b6fd3ed86b10644264841578/examples/multiComponent/contactAngle2d/contactAngle2d.cpp
// Upstream owns collision/coupling/wetting boundaries. Its three-point angle
// estimator is not used: closed native fields permit independent contour fits.
#define main openlb_upstream_contact_angle_main
#include "../examples/multiComponent/contactAngle2d/contactAngle2d.cpp"
#undef main
#include <nlohmann/json.hpp>
#include <algorithm>
#include <cmath>
#include <filesystem>
#include <fstream>
#include <iomanip>
#include <set>
#include <stdexcept>
#include "wetting_output.hpp"
#ifdef PLATFORM_GPU_HIP
#include "hip_identity.hpp"
constexpr Platform driver_platform = Platform::GPU_HIP;
constexpr const char* driver_backend = "hip";
#else
constexpr Platform driver_platform = Platform::CPU_SISD;
constexpr const char* driver_backend = "cpu";
#endif

using Json = nlohmann::json;

static double number(const Json& spec, const char* key) {
  if (!spec.at(key).is_number()) throw std::runtime_error("explicit finite numeric input required");
  const double value = spec.at(key).get<double>();
  if (!std::isfinite(value)) throw std::runtime_error("nonfinite wetting input rejected");
  return value;
}

static Json snapshot(MyCase& experiment, std::size_t step, double dx, double dt) {
  auto& geometry = experiment.getGeometry();
  auto& phase = experiment.getLattice(Component1{});
  auto& flow = experiment.getLattice(NavierStokes{});
  phase.setProcessingContext(ProcessingContext::Evaluation);
  flow.setProcessingContext(ProcessingContext::Evaluation);
  using PhaseD = MyCase::descriptor_t_of<Component1>;
  using FlowD = MyCase::descriptor_t_of<NavierStokes>;
  SuperLatticeDensity2D<double,PhaseD> phi(phase);
  SuperLatticeVelocity2D<double,FlowD> velocity(flow);
  const auto& load = phase.getLoadBalancer();
  if (load.size() != 1 || load.platform(0) != driver_platform || flow.getLoadBalancer().size() != 1 || flow.getLoadBalancer().platform(0) != driver_platform) throw std::runtime_error("one explicit block on the compiled platform required; silent CPU block assignment rejected");
  const auto& cuboid = geometry.getBlockGeometry(0).getCuboid();
  const int nx = cuboid.getNx(), ny = cuboid.getNy(), global = load.glob(0);
  std::string name = "wetting-" + std::to_string(step) + ".csv";
  if (std::filesystem::exists(name)) throw std::runtime_error("immutable wetting field already exists");
  std::ofstream output(name + ".partial");
  output << std::setprecision(17) << "x_m,y_m,material,phi,u_lattice,v_lattice\n";
  double amount = 0., minimum = 1., maximum = 0., max_speed = 0.;
  std::size_t fluid = 0;
  for (int y=0; y<ny; ++y) for (int x=0; x<nx; ++x) {
    int input[] = {global,x,y};
    const int material = geometry.get(global,x,y);
    double value{}, u[2]{};
    if (material != 1 && material != 2) throw std::runtime_error("unexpected native material in closed reference domain");
    if (!phi(&value,input) || !velocity(u,input) || !std::isfinite(value) || !std::isfinite(u[0]) || !std::isfinite(u[1])) throw std::runtime_error("nonfinite/missing wetting field");
    const auto position = geometry.getPhysR({global,x,y});
    output << position[0]*dx << ',' << position[1]*dx << ',' << material << ',' << value << ',' << u[0] << ',' << u[1] << '\n';
    if (material == 1) {
      amount += (1.-value)*dx*dx; // native droplet area per unit out-of-plane depth
      minimum = std::min(minimum,value); maximum = std::max(maximum,value);
      max_speed = std::max(max_speed,std::hypot(u[0],u[1])); ++fluid;
    }
  }
  output.flush(); if (!output) throw std::runtime_error("native scientific field write failed"); output.close();
  std::filesystem::rename(name + ".partial",name);
  if (fluid != geometry.getStatistics().getNvoxel(1) || amount<=0.) throw std::runtime_error("native bulk coverage/phase area failed");
  return { {"path",name}, {"step",step}, {"time_s",step*dt}, {"nodes",nx*ny}, {"fluid_nodes",fluid}, {"shape",{nx,ny}},
    {"droplet_area_m2",amount}, {"minimum_phi",minimum}, {"maximum_phi",maximum}, {"max_speed_lattice",max_speed} };
}

int main(int argc, char** argv) {
  try {
    if (argc!=3 || std::string(argv[1])!="reference") throw std::runtime_error("usage: harbor-cad-openlb-wetting reference request.json");
    if (std::filesystem::file_size(argv[2])>65536) throw std::runtime_error("bounded wetting descriptor required");
    Json spec; std::ifstream(argv[2]) >> spec;
    std::set<std::string> names={"schema_version","synthetic","backend","formulation","diameter_m","initial_center_above_wall_m","resolution","interface_width_m","density_liquid_kg_m3","density_vapor_kg_m3","viscosity_liquid_m2_s","viscosity_vapor_m2_s","surface_tension_n_m","contact_angle_deg","phase_relaxation_time","steps","observation_steps","mass_tolerance","angle_tolerance_deg","material_provenance","boundary_provenance"};
    // HIP descriptors carry their explicit PCI/UUID compute selection; CPU descriptors never do.
    if (std::string(driver_backend)=="hip") names.insert("selection");
    std::set<std::string> observed; for (auto it=spec.begin(); it!=spec.end(); ++it) observed.insert(it.key());
    if (observed!=names || spec.at("schema_version")!=1 || spec.at("synthetic")!=true || spec.at("backend")!=driver_backend || spec.at("formulation")!="well_balanced_contact_angle_2d") throw std::runtime_error(std::string("explicit synthetic 2D wetting reference descriptor for the compiled backend required: ")+driver_backend);
    if (number(spec,"initial_center_above_wall_m")!=0.) throw std::runtime_error("explicit wall-centered initial half-circle required; initial geometry must be invariant under refinement");
    const double diameter=number(spec,"diameter_m"), width=number(spec,"interface_width_m"), liquid=number(spec,"density_liquid_kg_m3"), vapor=number(spec,"density_vapor_kg_m3"), nu=number(spec,"viscosity_liquid_m2_s"), gas_nu=number(spec,"viscosity_vapor_m2_s"), sigma=number(spec,"surface_tension_n_m"), angle=number(spec,"contact_angle_deg"), mobility=number(spec,"phase_relaxation_time"), mass_gate=number(spec,"mass_tolerance"), angle_gate=number(spec,"angle_tolerance_deg");
    if (!spec.at("resolution").is_number_unsigned() || !spec.at("steps").is_number_unsigned()) throw std::runtime_error("integer refinement/step limits required");
    const int n=spec.at("resolution"); const std::size_t steps=spec.at("steps");
    const double dx=diameter/n, dt=(1.-0.5)/3.*dx*dx/nu;
    const double conversion_sigma=liquid*dx*dx*dx/(dt*dt), lattice_sigma=sigma/conversion_sigma, lattice_width=width/dx;
    if (n<24 || n>96 || steps<100 || steps>800000 || diameter<=0. || diameter>0.001 || width<=0. || width>diameter/6. || lattice_width<3. || lattice_width>16. || liquid<=0. || liquid!=vapor || nu<=0. || nu!=gas_nu || sigma<=0. || lattice_sigma>0.02 || !std::isfinite(lattice_sigma) || angle<60. || angle>120. || mobility<0.6 || mobility>1.5 || mass_gate<=0. || mass_gate>1e-3 || angle_gate<=0. || angle_gate>5.) throw std::runtime_error("bounded equal-density/equal-viscosity synthetic reference, resolved interface and unchanged acceptance required; real water/air ratios unqualified");
    for (const auto* key : {"material_provenance","boundary_provenance"}) if (!spec.at(key).is_string() || spec.at(key).get<std::string>().empty() || spec.at(key).get<std::string>().size()>4096) throw std::runtime_error("explicit synthetic material/boundary provenance required");
    auto retained=spec.at("observation_steps").get<std::vector<std::size_t>>();
    if (retained.size()<2 || retained.size()>32 || retained.front()!=0 || retained.back()!=steps || !std::is_sorted(retained.begin(),retained.end()) || std::adjacent_find(retained.begin(),retained.end())!=retained.end()) throw std::runtime_error("ordered distinct initial/final bounded native observations required");
    verify_wetting_output_directory(".");
    Json receipt={{"schema_version",1},{"adapter","OpenLB"},{"backend",driver_backend}};
#ifdef PLATFORM_GPU_HIP
    // Select and verify the card before any device allocation or scientific output.
    receipt.update(hip_select_device(spec.at("selection")));
    receipt["compiled_architecture"]="@hip_architecture@";
    receipt["uuid_source"]="hipDeviceGetUuid; GPU- followed by 16 bytes in lowercase hex";
    int runtimeVersion=0, driverVersion=0;
    hip_check(hipRuntimeGetVersion(&runtimeVersion)); hip_check(hipDriverGetVersion(&driverVersion));
    receipt["hip_runtime_version"]=runtimeVersion; receipt["hip_driver_version"]=driverVersion; receipt["compiled_hip_version"]=HIP_VERSION;
#endif
    initialize(&argc,&argv); singleton::directories().setOutputDir("./tmp/");
    MyCase::ParametersD params;
    using namespace olb::parameters;
    params.set<DOMAIN_EXTENT>({2.5*n,1.5*n}); params.set<RESOLUTION>(n); params.set<OVERLAP>(2);
    params.set<LATTICE_RELAXATION_TIME>(1.); params.set<LATTICE_RELAXATION_TIME_2>(1.); params.set<LATTICE_RELAXATION_TIME_PF>(mobility);
    params.set<PHYS_CHAR_LENGTH>(diameter); params.set<C_RHO>(liquid); params.set<NU_LIQUID>(nu);
    params.set<RHO_LIQUID>(1.); params.set<RHO_VAPOR>(vapor/liquid); params.set<SURFACE_TENSION>(lattice_sigma); params.set<parameters::INTERFACE_WIDTH>(lattice_width); params.set<parameters::THETA>(angle);
    Mesh mesh=createMesh(params); MyCase experiment(params,mesh);
    prepareGeometry(experiment); prepareLattice(experiment); setInitialValues(experiment);
    auto& flow=experiment.getLattice(NavierStokes{}); auto& phase=experiment.getLattice(Component1{});
#ifdef PLATFORM_GPU_HIP
    // Upstream setInitialValues runs PreCoupling/ChemPotCalc before initialize()
    // uploads host state, then re-equilibrates phase populations on the host. On
    // a device lattice that never reaches the CPU initial state. Replay the CPU
    // order explicitly: upload host state, run the same device stages and the
    // PostStream pass of initialize(), then upload the host equilibrium populations.
    flow.setProcessingContext(ProcessingContext::Simulation); phase.setProcessingContext(ProcessingContext::Simulation);
    phase.executePostProcessors(stage::PreCoupling()); phase.getCommunicator(stage::PreCoupling()).communicate();
    phase.executePostProcessors(stage::ChemPotCalc()); phase.getCommunicator(stage::PreCoupling()).communicate();
    flow.getBlock(0).postProcess(); phase.getBlock(0).postProcess();
    phase.setProcessingContext<Array<descriptors::POPULATION>>(ProcessingContext::Simulation);
    phase.getCommunicator(stage::PreCoupling()).communicate();
#endif
    const auto& converter=flow.getUnitConverter();
    if (std::abs(converter.getPhysDeltaT()/dt-1.)>1e-12 || std::abs(converter.getPhysDeltaX()/dx-1.)>1e-12) throw std::runtime_error("independent SI conversion differs from pinned converter");
    Json snapshots=Json::array();
    for (std::size_t i=0; i<=steps; ++i) {
      if (std::binary_search(retained.begin(),retained.end(),i)) {
        snapshots.push_back(snapshot(experiment,i,dx,dt));
        // Device Simulation context uploads host mirrors. Re-enter only after an
        // observation, never each step: repeated uploads erase evolving device state.
        if (i<steps) {flow.setProcessingContext(ProcessingContext::Simulation); phase.setProcessingContext(ProcessingContext::Simulation);}
      }
      if (i==steps) break;
      flow.collideAndStream(); phase.collideAndStream();
      phase.getCommunicator(stage::PreCoupling()).communicate(); phase.executePostProcessors(stage::PreCoupling());
      phase.getCommunicator(stage::PreCoupling()).communicate(); phase.executePostProcessors(stage::ChemPotCalc());
      phase.getCommunicator(stage::PreCoupling()).communicate(); experiment.getOperator("Coupling").apply();
    }
    double mass_error=0.; const double initial=snapshots.front().at("droplet_area_m2");
    for (const auto& item:snapshots) mass_error=std::max(mass_error,std::abs(item.at("droplet_area_m2").get<double>()/initial-1.));
#ifdef PLATFORM_GPU_HIP
    hip_check(hipDeviceSynchronize()); hip_check(hipGetLastError());
    int current=-1; hip_check(hipGetDevice(&current));
    if (current!=receipt.at("ordinal_diagnostic_only")) throw std::runtime_error("HIP execution device changed");
    receipt["gpu_blocks"]=phase.getLoadBalancer().size();
    receipt["gpu_kernel_completion_verified"]=true;
    receipt["processing_context_policy"]="device-resident stepping; host synchronization at retained observations";
#endif
    receipt.update(Json{{"executed",true},{"software_fallback",false},{"source_revision","145cd54810b468f4b6fd3ed86b10644264841578"},{"formulation",spec.at("formulation")},{"dimensionality",2},{"precision","float64"},{"synthetic",true},{"request",spec},{"spacing_m",dx},{"physical_step_s",dt},{"interface_width_lattice",lattice_width},{"surface_tension_lattice",lattice_sigma},{"wall_y_m",0.5*dx},{"snapshots",snapshots},{"mass_relative_error",mass_error},{"mass_tolerance",mass_gate},{"numerical_verification","mass checked; independent contact-angle/refinement gate required"},{"physical_validation","unqualified"},{"scope","static synthetic diffuse-interface planar wetting; no water-air ratio, inlet splash, ingress, evaporation or retention conclusion"}});
    std::ofstream output("wetting-receipt.json.partial"); output<<receipt.dump(2); output.flush(); if (!output) throw std::runtime_error("receipt write failed"); output.close(); std::filesystem::rename("wetting-receipt.json.partial","wetting-receipt.json");
    if (mass_error>mass_gate) throw std::runtime_error("approved phase-mass gate failed; original fields retained");
    return 0;
  } catch(const std::exception& e) {std::cerr<<e.what()<<std::endl;return 1;}
}
